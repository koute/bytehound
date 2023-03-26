use std::cmp::{max, min};
use std::io::{self, Read, Write};

use ahash::AHashMap as HashMap;
use std::collections::hash_map::Entry;

use common::speedy::Writable;
use common::Timestamp;

use common::event::{AllocBody, AllocationId, Event};

use crate::loader::Loader;
use crate::threaded_lz4_stream::Lz4Writer;

use crate::reader::parse_events;

struct Allocation {
    counter: u64,
    events: smallvec::SmallVec<[BufferedAllocation; 1]>,
}

impl Allocation {
    pub fn new(counter: u64) -> Self {
        Allocation {
            counter,
            events: Default::default(),
        }
    }
}

struct BufferedAllocation {
    timestamp: Timestamp,
    allocation: AllocBody,
}

fn emit(
    id: AllocationId,
    mut events: smallvec::SmallVec<[BufferedAllocation; 1]>,
    fp: &mut impl Write,
) -> Result<(), std::io::Error> {
    if events.len() == 0 {
        return Ok(());
    }

    let mut iter = events.drain(..);

    let BufferedAllocation {
        timestamp,
        allocation,
    } = iter.next().unwrap();
    let mut old_pointer = allocation.pointer;
    Event::AllocEx {
        id,
        timestamp,
        allocation,
    }
    .write_to_stream(&mut *fp)?;

    while let Some(BufferedAllocation {
        timestamp,
        allocation,
    }) = iter.next()
    {
        let new_pointer = allocation.pointer;
        Event::ReallocEx {
            id,
            timestamp,
            old_pointer,
            allocation,
        }
        .write_to_stream(&mut *fp)?;
        old_pointer = new_pointer;
    }

    Ok(())
}

struct GroupStatistics {
    first_allocation: Timestamp,
    last_allocation: Timestamp,
    free_count: u64,
    free_size: u64,
    min_size: u64,
    max_size: u64,
}

pub fn squeeze_data<F, G>(
    input_fp: F,
    output_fp: G,
    threshold: Option<u64>,
) -> Result<(), io::Error>
where
    F: Read + Send + 'static,
    G: Write + Send + 'static,
{
    let (header, event_stream) = parse_events(input_fp)?;

    let mut current_timestamp = header.initial_timestamp;
    let mut ofp = Lz4Writer::new(output_fp);
    Event::Header(header).write_to_stream(&mut ofp)?;
    let threshold = threshold.map(Timestamp::from_secs);

    {
        let mut previous_backtrace_on_thread = HashMap::new();
        let mut backtrace_cache: HashMap<Vec<u64>, u64> = Default::default();
        let mut backtrace_map: HashMap<u64, u64> = Default::default();
        let mut stats_by_backtrace: HashMap<u64, GroupStatistics> = Default::default();
        let mut young_allocations_by_id: HashMap<AllocationId, Allocation> = Default::default();
        let mut mature_allocations_by_id: HashMap<AllocationId, Allocation> = Default::default();
        let mut allocations_by_pointer: HashMap<u64, Allocation> = Default::default();
        let mut last_flush = current_timestamp;
        let mut flushed_buffer = Vec::new();
        let mut allocation_counter = 0;

        for event in event_stream {
            let event = event?;
            let mut event = match event {
                Event::Alloc {
                    timestamp,
                    allocation,
                } => Event::AllocEx {
                    id: AllocationId::UNTRACKED,
                    timestamp,
                    allocation,
                },
                Event::Realloc {
                    timestamp,
                    old_pointer,
                    allocation,
                } => Event::ReallocEx {
                    id: AllocationId::UNTRACKED,
                    timestamp,
                    old_pointer,
                    allocation,
                },
                Event::Free {
                    timestamp,
                    pointer,
                    backtrace,
                    thread,
                } => Event::FreeEx {
                    id: AllocationId::UNTRACKED,
                    timestamp,
                    pointer,
                    backtrace,
                    thread,
                },
                event => event,
            };

            if let Some(threshold) = threshold {
                if current_timestamp - last_flush > threshold {
                    std::mem::swap(&mut young_allocations_by_id, &mut mature_allocations_by_id);
                    let mut allocations_kept = 0;
                    for (id, allocation) in young_allocations_by_id.drain() {
                        if allocation.events[0].timestamp < current_timestamp
                            && current_timestamp - allocation.events[0].timestamp >= threshold
                        {
                            flushed_buffer.push((id, allocation));
                        } else {
                            // This should not happen, but let's handle it just in case.
                            mature_allocations_by_id.insert(id, allocation);
                            allocations_kept += 1;
                        }
                    }

                    if allocations_kept > 0 {
                        info!("Was unable to flush {} allocation(s)", allocations_kept);
                    }
                    last_flush = current_timestamp;

                    // Sort it so that the output doesn't differ based on the hashmap's iteration order.
                    flushed_buffer.sort_unstable_by_key(|(_, allocation)| allocation.counter);

                    for (id, allocation) in flushed_buffer.drain(..) {
                        emit(id, allocation.events, &mut ofp)?;
                    }
                }
            }

            match event {
                Event::Alloc { .. } | Event::Realloc { .. } | Event::Free { .. } => unreachable!(),
                Event::Backtrace { id, ref addresses } => {
                    let addresses = addresses.clone().into_owned();
                    let new_id = backtrace_cache.entry(addresses).or_insert(id);
                    backtrace_map.insert(id, *new_id);
                    if id != *new_id {
                        continue;
                    }
                }
                Event::Backtrace32 { id, ref addresses } => {
                    let addresses = addresses.iter().map(|&p| p as u64).collect();
                    let new_id = backtrace_cache.entry(addresses).or_insert(id);
                    backtrace_map.insert(id, *new_id);
                    if id != *new_id {
                        continue;
                    }
                }
                Event::PartialBacktrace {
                    id,
                    thread,
                    frames_invalidated,
                    ref mut addresses,
                } => {
                    let addresses = Loader::expand_partial_backtrace(
                        &mut previous_backtrace_on_thread,
                        thread,
                        frames_invalidated,
                        addresses.iter().cloned(),
                    );
                    *previous_backtrace_on_thread.get_mut(&thread).unwrap() = addresses.clone();

                    let new_id = backtrace_cache.entry(addresses.clone()).or_insert(id);
                    backtrace_map.insert(id, *new_id);
                    if id != *new_id {
                        continue;
                    }

                    let event = Event::Backtrace {
                        id,
                        addresses: addresses.into(),
                    };
                    event.write_to_stream(&mut ofp)?;

                    continue;
                }
                Event::PartialBacktrace32 {
                    id,
                    thread,
                    frames_invalidated,
                    ref mut addresses,
                } => {
                    let addresses = Loader::expand_partial_backtrace(
                        &mut previous_backtrace_on_thread,
                        thread,
                        frames_invalidated,
                        addresses.iter().map(|&address| address as u64),
                    );
                    *previous_backtrace_on_thread.get_mut(&thread).unwrap() = addresses.clone();

                    let new_id = backtrace_cache.entry(addresses.clone()).or_insert(id);
                    backtrace_map.insert(id, *new_id);
                    if id != *new_id {
                        continue;
                    }

                    let event = Event::Backtrace {
                        id,
                        addresses: addresses.into(),
                    };
                    event.write_to_stream(&mut ofp)?;

                    continue;
                }
                Event::AllocEx {
                    mut allocation,
                    timestamp,
                    id,
                    ..
                } => {
                    current_timestamp = std::cmp::max(timestamp, current_timestamp);

                    let usable_size = allocation.size + allocation.extra_usable_space as u64;
                    {
                        allocation.backtrace =
                            backtrace_map.get(&allocation.backtrace).copied().unwrap();
                        let stats = stats_by_backtrace
                            .entry(allocation.backtrace)
                            .or_insert_with(|| GroupStatistics {
                                first_allocation: timestamp,
                                last_allocation: timestamp,
                                free_count: 0,
                                free_size: 0,
                                min_size: usable_size,
                                max_size: usable_size,
                            });

                        stats.first_allocation = min(stats.first_allocation, timestamp);
                        stats.last_allocation = max(stats.last_allocation, timestamp);
                        stats.min_size = min(stats.min_size, usable_size);
                        stats.max_size = min(stats.max_size, usable_size);
                    }

                    let entry;
                    if !id.is_invalid() && !id.is_untracked() {
                        entry = match young_allocations_by_id.entry(id) {
                            Entry::Vacant(entry) => {
                                let counter = allocation_counter;
                                allocation_counter += 1;
                                let allocation = Allocation::new(counter);
                                entry.insert(allocation)
                            }
                            Entry::Occupied(..) => {
                                warn!("Duplicate allocation with ID: {:?}", id);
                                continue;
                            }
                        };
                    } else {
                        entry = match allocations_by_pointer.entry(allocation.pointer) {
                            Entry::Vacant(entry) => {
                                let counter = allocation_counter;
                                allocation_counter += 1;
                                let allocation = Allocation::new(counter);
                                entry.insert(allocation)
                            }
                            Entry::Occupied(..) => {
                                warn!(
                                    "Duplicate allocation with address: 0x{:016X}",
                                    allocation.pointer
                                );
                                continue;
                            }
                        };
                    }

                    entry.events.push(BufferedAllocation {
                        timestamp,
                        allocation,
                    });
                    continue;
                }
                Event::ReallocEx {
                    timestamp,
                    mut allocation,
                    old_pointer,
                    id,
                    ..
                } => {
                    let usable_size = allocation.size + allocation.extra_usable_space as u64;
                    {
                        allocation.backtrace =
                            backtrace_map.get(&allocation.backtrace).copied().unwrap();
                        let stats = stats_by_backtrace
                            .entry(allocation.backtrace)
                            .or_insert_with(|| GroupStatistics {
                                first_allocation: timestamp,
                                last_allocation: timestamp,
                                free_count: 0,
                                free_size: 0,
                                min_size: usable_size,
                                max_size: usable_size,
                            });

                        stats.first_allocation = min(stats.first_allocation, timestamp);
                        stats.last_allocation = max(stats.last_allocation, timestamp);
                        stats.min_size = min(stats.min_size, usable_size);
                        stats.max_size = min(stats.max_size, usable_size);
                    }

                    let entry;
                    if !id.is_invalid() && !id.is_untracked() {
                        entry = match young_allocations_by_id.get_mut(&id) {
                            Some(entry) => entry,
                            None => match mature_allocations_by_id.get_mut(&id) {
                                Some(entry) => entry,
                                None => {
                                    let event = Event::ReallocEx {
                                        timestamp,
                                        allocation,
                                        old_pointer,
                                        id,
                                    };
                                    event.write_to_stream(&mut ofp)?;
                                    continue;
                                }
                            },
                        };
                    } else {
                        let old_entry = match allocations_by_pointer.remove(&old_pointer) {
                            Some(entry) => entry,
                            None => {
                                warn!("Invalid reallocation of address: 0x{:016X}", old_pointer);
                                continue;
                            }
                        };

                        entry = match allocations_by_pointer.entry(allocation.pointer) {
                            Entry::Vacant(entry) => entry.insert(old_entry),
                            Entry::Occupied(..) => {
                                warn!(
                                    "Duplicate reallocation with address: 0x{:016X}",
                                    allocation.pointer
                                );
                                continue;
                            }
                        };
                    }

                    entry.events.push(BufferedAllocation {
                        timestamp,
                        allocation,
                    });
                    continue;
                }
                Event::FreeEx {
                    id,
                    timestamp,
                    pointer,
                    backtrace,
                    thread,
                } => {
                    let mut entry;
                    if !id.is_invalid() && !id.is_untracked() {
                        entry = young_allocations_by_id.remove(&id);
                        if entry.is_none() {
                            entry = mature_allocations_by_id.remove(&id);
                        }

                        if entry.is_none() {
                            let event = Event::FreeEx {
                                id,
                                timestamp,
                                pointer,
                                backtrace,
                                thread,
                            };
                            event.write_to_stream(&mut ofp)?;
                            continue;
                        }
                    } else {
                        entry = allocations_by_pointer.remove(&pointer);
                    }

                    if let Some(entry) = entry {
                        if timestamp < entry.events[0].timestamp {
                            warn!("Deallocation in the past of address: 0x{:016X}", pointer);
                        } else {
                            if let Some(threshold) = threshold {
                                let lifetime = timestamp - entry.events[0].timestamp;
                                if lifetime > threshold {
                                    emit(id, entry.events, &mut ofp)?;
                                    let event = Event::FreeEx {
                                        id,
                                        timestamp,
                                        pointer,
                                        backtrace,
                                        thread,
                                    };
                                    event.write_to_stream(&mut ofp)?;
                                    continue;
                                }
                            }
                        }

                        for buffered in entry.events {
                            let usable_size = buffered.allocation.size
                                + buffered.allocation.extra_usable_space as u64;
                            let stats = stats_by_backtrace
                                .get_mut(&buffered.allocation.backtrace)
                                .unwrap();
                            stats.free_count += 1;
                            stats.free_size += usable_size;
                        }
                    }

                    continue;
                }
                Event::MemoryMap {
                    ref mut backtrace, ..
                }
                | Event::MemoryUnmap {
                    ref mut backtrace, ..
                }
                | Event::Mallopt {
                    ref mut backtrace, ..
                } => {
                    *backtrace = backtrace_map.get(backtrace).copied().unwrap();
                }

                Event::GroupStatistics {
                    ref mut backtrace,
                    first_allocation,
                    last_allocation,
                    free_count,
                    free_size,
                    min_size,
                    max_size,
                } => {
                    {
                        *backtrace = backtrace_map.get(backtrace).copied().unwrap();
                        let stats = stats_by_backtrace.entry(*backtrace).or_insert_with(|| {
                            GroupStatistics {
                                first_allocation,
                                last_allocation,
                                free_count: 0,
                                free_size: 0,
                                min_size,
                                max_size,
                            }
                        });

                        stats.first_allocation = min(stats.first_allocation, first_allocation);
                        stats.last_allocation = max(stats.last_allocation, last_allocation);
                        stats.min_size = min(stats.min_size, min_size);
                        stats.max_size = max(stats.max_size, max_size);
                        stats.free_count += free_count;
                        stats.free_size += free_size;
                    }

                    continue;
                }

                Event::File { .. } => {}
                Event::File64 { .. } => {}
                Event::Header { .. } => {}
                Event::MemoryDump { .. } => {}
                Event::Marker { .. } => {}
                Event::Environ { .. } => {}
                Event::WallClock { .. } => {}
                Event::String { .. } => {}
                Event::DecodedFrame { .. } => {}
                Event::DecodedBacktrace { .. } => {}
                Event::MemoryMapEx { .. } => {}
                Event::AddRegion { .. } => {}
                Event::RemoveRegion { .. } => {}
                Event::UpdateRegionUsage { .. } => {}
            }

            event.write_to_stream(&mut ofp)?;
        }

        for (id, bucket) in mature_allocations_by_id {
            emit(id, bucket.events, &mut ofp)?;
        }

        for (id, bucket) in young_allocations_by_id {
            emit(id, bucket.events, &mut ofp)?;
        }

        for (_, bucket) in allocations_by_pointer {
            emit(
                common::event::AllocationId::UNTRACKED,
                bucket.events,
                &mut ofp,
            )?;
        }

        for (backtrace, stats) in stats_by_backtrace {
            let event = Event::GroupStatistics {
                backtrace,
                first_allocation: stats.first_allocation,
                last_allocation: stats.last_allocation,
                free_count: stats.free_count,
                free_size: stats.free_size,
                min_size: stats.min_size,
                max_size: stats.max_size,
            };
            event.write_to_stream(&mut ofp)?;
        }
    }

    ofp.flush()?;

    Ok(())
}
