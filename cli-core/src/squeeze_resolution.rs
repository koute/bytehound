use std::io::{self, Read, Write};
use std::fs::{self, File};
use std::path::Path;
use std::cmp::{max, min};
use std::mem;
use std::u32;
use std::path::PathBuf;
use std::collections::LinkedList;
use std::collections::HashSet;

use hashbrown::HashMap;

use common::Timestamp;
use common::speedy::{
    Writable
};

use common::event::{
    Event,
    AllocBody
};

use crate::loader::Loader;
use crate::threaded_lz4_stream::Lz4Writer;

use crate::reader::parse_events;

struct Allocation {
    counter: u64,
    backtrace: u64,
    usable_size: u64
}

struct GroupStatistics {
    first_allocation: Timestamp,
    last_allocation: Timestamp,
    free_count: u64,
    free_size: u64,
    min_size: u64,
    max_size: u64
}

#[derive(Clone,Default)]
struct BucketStatistics {
    current_size: u64,
    max_size: u64
}

fn get_timestamp( event: &Event ) -> Option<Timestamp>
{
    match event {
        Event::Alloc { timestamp, .. } |
        Event::Realloc { timestamp, .. } |
        Event::Free { timestamp, .. } |
        Event::File { timestamp, .. } |
        Event::MemoryMap { timestamp, .. } |
        Event::MemoryUnmap { timestamp, .. } |
        Event::Mallopt { timestamp, .. } |
        Event::WallClock { timestamp, .. } => {
            Some( *timestamp )
        },
        _ => {
            None
        }
    }
}

/*
    Takes stream of profiling data and reduces resolution of the eventualy freed allocations, thus saving memory.
    Leaked allocations are kept intact with full resolution.
*/
pub fn squeeze_data_resolution< G >( input: &PathBuf, output_fp: G, tmpfile_path: &Path, bucket_count: u32 ) -> Result< (), io::Error >
    where G: Write + Send + 'static
{
    /*
        Step #1: Find range of timestamps the data covers
        Step #2: Split freed allocation statistics into multiple buckets
        Step #3: Combine low-resolution bucketed data with original resolution leaked allocations
    */

    // Step #1: Stream => Min/Max timestamp

    let (timestamp_min, timestamp_max, timestamp_step) = {
        let mut timestamp_min: Option<Timestamp> = None; // Timestamp
        let mut timestamp_max: Option<Timestamp> = None;

        let (header, event_stream) = parse_events( File::open( &input )? )?;
        for event in event_stream {
            let event = event?;
            match event {
                Event::Alloc { timestamp, .. } |
                Event::Realloc { timestamp, .. } |
                Event::Free { timestamp, .. } |
                Event::File { timestamp, .. } |
                Event::MemoryMap { timestamp, .. } |
                Event::MemoryUnmap { timestamp, .. } |
                Event::Mallopt { timestamp, .. } |
                Event::WallClock { timestamp, .. } => {
                    if let Some( old_timestamp ) = timestamp_min {
                        if timestamp < old_timestamp {
                            timestamp_min = Some( timestamp );
                        }
                    } else {
                        timestamp_min = Some( timestamp );
                    }

                    if let Some( old_timestamp ) = timestamp_max {
                        if timestamp > old_timestamp {
                            timestamp_max = Some( timestamp );
                        }
                    } else {
                        timestamp_max = Some( timestamp );
                    }
                },
                Event::GroupStatistics { first_allocation, last_allocation, .. } => {
                    if let Some( old_timestamp ) = timestamp_min {
                        if first_allocation < old_timestamp {
                            timestamp_min = Some( first_allocation );
                        }
                    } else {
                        timestamp_min = Some( first_allocation );
                    }

                    if let Some( old_timestamp ) = timestamp_max {
                        if last_allocation > old_timestamp {
                            timestamp_max = Some( last_allocation );
                        }
                    } else {
                        timestamp_max = Some( last_allocation );
                    }
                },
                _ => {}
            }
        }

        let timestamp_min = match timestamp_min {
            Some (timestamp) => { timestamp },
            _ => { Timestamp::min() }
        };
        let timestamp_max = match timestamp_max {
            Some (timestamp) => { timestamp },
            _ => { Timestamp::max() }
        };
        assert!(timestamp_min < timestamp_max);
        (timestamp_min, timestamp_max, (timestamp_max - timestamp_min) / (bucket_count as f64))
    };
        

    // Step #2: Stream => Buckets

    let (header, event_stream) = parse_events( File::open( &input )? )?;

    let tfp = File::create( tmpfile_path )?;
    let mut tfp = Lz4Writer::new( tfp );
    Event::Header( header ).write_to_stream( &mut tfp )?;

    let mut buckets : LinkedList< HashMap< u64, BucketStatistics > > = Default::default();    

    let (live_allocations, mut stats_by_backtrace) = {
        let mut counter = 0;

        let mut previous_backtrace_on_thread = HashMap::new();
        let mut backtrace_cache: HashMap< Vec< u64 >, u64 > = Default::default();
        let mut backtrace_map: lru::LruCache< u64, u64 > = lru::LruCache::new( 128 );
        let mut stats_by_backtrace: HashMap< u64, GroupStatistics > = Default::default();
        let mut allocations: HashMap< u64, Allocation > = Default::default();
        let mut remap_backtraces = false;
        let mut current_bucket : HashMap< u64, BucketStatistics > = Default::default();
        let mut next_bucket_timestamp = timestamp_min + timestamp_step;

        for event in event_stream {
            let mut event = event?;

            match get_timestamp(&event) {
                Some( timestamp ) => {
                    if timestamp >= next_bucket_timestamp {
                        buckets.push_back(current_bucket.clone());
                        next_bucket_timestamp = next_bucket_timestamp + timestamp_step;
                        for bs in current_bucket.values_mut() {
                            bs.max_size = bs.current_size;
                        }
                    }                
                },
                _ => {}
            }

            match event {
                Event::Backtrace { id, ref addresses } => {
                    let addresses = addresses.clone().into_owned();
                    let new_id = backtrace_cache.entry( addresses ).or_insert( id );

                    backtrace_map.put( id, *new_id );
                    remap_backtraces = true;

                    if id != *new_id {
                        continue;
                    }
                },
                Event::PartialBacktrace { id, thread, frames_invalidated, ref mut addresses } => {
                    let addresses = Loader::expand_partial_backtrace( &mut previous_backtrace_on_thread, thread, frames_invalidated, addresses.iter().cloned() );
                    mem::replace( previous_backtrace_on_thread.get_mut( &thread ).unwrap(), addresses.clone() );

                    let new_id = backtrace_cache.entry( addresses.clone() ).or_insert( id );

                    backtrace_map.put( id, *new_id );
                    remap_backtraces = true;

                    if id != *new_id {
                        continue;
                    }

                    let event = Event::Backtrace { id, addresses: addresses.into() };
                    event.write_to_stream( &mut tfp )?;

                    continue;
                },
                Event::PartialBacktrace32 { id, thread, frames_invalidated, ref mut addresses } => {
                    let addresses = Loader::expand_partial_backtrace( &mut previous_backtrace_on_thread, thread, frames_invalidated, addresses.iter().map( |&address| address as u64 ) );
                    mem::replace( previous_backtrace_on_thread.get_mut( &thread ).unwrap(), addresses.clone() );

                    let new_id = backtrace_cache.entry( addresses.clone() ).or_insert( id );

                    backtrace_map.put( id, *new_id );
                    remap_backtraces = true;

                    if id != *new_id {
                        continue;
                    }

                    let event = Event::Backtrace { id, addresses: addresses.into() };
                    event.write_to_stream( &mut tfp )?;

                    continue;
                },
                Event::Alloc { allocation: AllocBody { ref mut backtrace, pointer, size, extra_usable_space, .. }, timestamp, .. } => {
                    let usable_size = size + extra_usable_space as u64;
                    {
                        if remap_backtraces {
                            *backtrace = backtrace_map.get( backtrace ).cloned().unwrap();
                        }

                        let stats = stats_by_backtrace.entry( *backtrace ).or_insert_with( || {
                            GroupStatistics {
                                first_allocation: timestamp,
                                last_allocation: timestamp,
                                free_count: 0,
                                free_size: 0,
                                min_size: usable_size,
                                max_size: usable_size
                            }
                        });

                        stats.first_allocation = min( stats.first_allocation, timestamp );
                        stats.last_allocation = max( stats.last_allocation, timestamp );
                        stats.free_size += usable_size;
                        stats.min_size = min( stats.min_size, usable_size );
                        stats.max_size = min( stats.max_size, usable_size );
                    }

                    allocations.insert( pointer, Allocation { counter, backtrace: *backtrace, usable_size } );
                    counter += 1;

                    {
                        let stats = current_bucket.entry( *backtrace ).or_insert_with( || {
                            BucketStatistics {
                                current_size: 0,
                                max_size: 0
                            }
                        });
                        stats.current_size += usable_size;
                        if stats.current_size > stats.max_size {
                            stats.max_size = stats.current_size;
                        }
                    }
                },
                Event::Realloc { timestamp, mut allocation, old_pointer, .. } => {
                    let usable_size = allocation.size + allocation.extra_usable_space as u64;
                    {
                        if remap_backtraces {
                            allocation.backtrace = backtrace_map.get( &allocation.backtrace ).cloned().unwrap();
                        }

                        let stats = stats_by_backtrace.entry( allocation.backtrace ).or_insert_with( || {
                            GroupStatistics {
                                first_allocation: timestamp,
                                last_allocation: timestamp,
                                free_count: 0,
                                free_size: 0,
                                min_size: usable_size,
                                max_size: usable_size
                            }
                        });

                        stats.first_allocation = min( stats.first_allocation, timestamp );
                        stats.last_allocation = max( stats.last_allocation, timestamp );
                        stats.free_size += usable_size;
                        stats.min_size = min( stats.min_size, usable_size );
                        stats.max_size = min( stats.max_size, usable_size );
                    }

                    if let Some( old_allocation ) = allocations.remove( &old_pointer ) {
                        {
                            // We forget old allocation data
                            let stats = current_bucket.entry( old_allocation.backtrace ).or_insert_with( || {
                                BucketStatistics {
                                    current_size: 0,
                                    max_size: 0
                                }
                            });
                            stats.current_size -= usable_size;
                        }

                        if let Some( stats ) = stats_by_backtrace.get_mut( &old_allocation.backtrace ) {
                            stats.free_count += 1;
                        }
                    }

                    allocations.insert( allocation.pointer, Allocation { counter, backtrace: allocation.backtrace, usable_size } );

                    let event = Event::Alloc { timestamp, allocation: allocation.clone() };
                    event.write_to_stream( &mut tfp )?;

                    counter += 1;

                    {
                        // We add new allocation data
                        let stats = current_bucket.entry( allocation.backtrace ).or_insert_with( || {
                            BucketStatistics {
                                current_size: 0,
                                max_size: 0
                            }
                        });
                        stats.current_size += usable_size;
                        if stats.current_size > stats.max_size {
                            stats.max_size = stats.current_size;
                        }
                    }

                    continue;
                },
                Event::Free { pointer, .. } => {
                    if let Some( allocation ) = allocations.remove( &pointer ) {
                        
                        {
                            let stats = current_bucket.entry( allocation.backtrace ).or_insert_with( || {
                                BucketStatistics {
                                    current_size: 0,
                                    max_size: 0
                                }
                            });
                            stats.current_size -= allocation.usable_size;
                        }
                        
                        if let Some( stats ) = stats_by_backtrace.get_mut( &allocation.backtrace ) {
                            stats.free_count += 1;
                        }
                    }

                    continue;
                },
                Event::MemoryMap { ref mut backtrace, length, .. } => {
                    if remap_backtraces {
                        *backtrace = backtrace_map.get( backtrace ).cloned().unwrap();
                    }

                    {
                        let stats = current_bucket.entry( *backtrace ).or_insert_with( || {
                            BucketStatistics {
                                current_size: 0,
                                max_size: 0
                            }
                        });
                        stats.current_size += length;
                        if stats.current_size > stats.max_size {
                            stats.max_size = stats.current_size;
                        }
                    }
                },
                Event::MemoryUnmap { ref mut backtrace, length, .. } => {
                    if remap_backtraces {
                        *backtrace = backtrace_map.get( backtrace ).cloned().unwrap();
                    }

                    {
                        let stats = current_bucket.entry( *backtrace ).or_insert_with( || {
                            BucketStatistics {
                                current_size: 0,
                                max_size: 0
                            }
                        });
                        stats.current_size -= length;
                    }
                },
                Event::Mallopt { ref mut backtrace, .. } => {
                    if remap_backtraces {
                        *backtrace = backtrace_map.get( backtrace ).cloned().unwrap();
                    }
                },

                Event::GroupStatistics { ref mut backtrace, first_allocation, last_allocation, free_count, free_size, min_size, max_size } => {
                    {
                        if remap_backtraces {
                            *backtrace = backtrace_map.get( backtrace ).cloned().unwrap();
                        }
                        let stats = stats_by_backtrace.entry( *backtrace ).or_insert_with( || {
                            GroupStatistics {
                                first_allocation,
                                last_allocation,
                                free_count: 0,
                                free_size: 0,
                                min_size,
                                max_size
                            }
                        });

                        stats.first_allocation = min( stats.first_allocation, first_allocation );
                        stats.last_allocation = max( stats.last_allocation, last_allocation );
                        stats.min_size = min( stats.min_size, min_size );
                        stats.max_size = max( stats.max_size, max_size );
                        stats.free_count += free_count;
                        stats.free_size += free_size;
                    }

                    continue;
                },

                Event::File { .. } => {},
                Event::Header { .. } => {},
                Event::MemoryDump { .. } => {},
                Event::Marker { .. } => {},
                Event::Environ { .. } => {},
                Event::WallClock { .. } => {},
                Event::String { .. } => {},
                Event::DecodedFrame { .. } => {},
                Event::DecodedBacktrace { .. } => {}
            }

            event.write_to_stream( &mut tfp )?;
        }

        let live_allocations: HashMap< _, _ > = allocations.into_iter().map( |(pointer, allocation)| {
            stats_by_backtrace.get_mut( &allocation.backtrace ).unwrap().free_size -= allocation.usable_size;
            (pointer, allocation.counter)
        }).collect();
        (live_allocations, stats_by_backtrace)
    };

    tfp.flush()?;
    mem::drop( tfp );

    // Step #3: Buckets+Leaks => Stream

    let ifp = File::open( tmpfile_path )?;
    let (header, event_stream) = parse_events( ifp )?;
    let mut ofp = Lz4Writer::new( output_fp );
    Event::Header( header ).write_to_stream( &mut ofp )?;

    {
        let mut counter = 0;
        let mut last_decoded_backtrace_id = 0;
        let mut next_bucket_timestamp = timestamp_min;
        buckets.push_front(Default::default()); // we start with no allocations
        let mut used_backtraces : HashSet< u64 > = Default::default();
        for event in event_stream {
            let event = event?;
            let mut backtrace_id = None;

            match get_timestamp(&event) {
                Some( timestamp ) => {
                    if timestamp >= next_bucket_timestamp {
                        let old_bucket = buckets.pop_front().unwrap();
                        //let new_bucket = buckets.front().unwrap();
                        // remove previous bucket allocations
                        for bt in &used_backtraces {
                            let event = Event::Free { 
                                timestamp: timestamp,
                                pointer: *bt,
                                backtrace: *bt,
                                thread: 1234
                            };
                            event.write_to_stream( &mut ofp )?;
                        }
                        // add new bucket allocations
                        {
                            for bt in old_bucket.keys() {
                                let stats = old_bucket.get(bt).unwrap_or(
                                    &BucketStatistics {
                                        current_size: 0,
                                        max_size: 0
                                    }
                                );
                                let event = Event::Alloc { timestamp, allocation: AllocBody{
                                    // TODO: we use backtrace ID as address, which will possibly work,
                                    // but can overlap with actual addresses
                                    pointer: *bt,
                                    size: stats.max_size,
                                    backtrace: *bt,
                                    thread: 1234,
                                    flags: 0,
                                    extra_usable_space: 0,
                                    preceding_free_space: 0
                                } };
                                event.write_to_stream( &mut ofp )?;
                                used_backtraces.insert(*bt);
                            }
                        }

                        next_bucket_timestamp = next_bucket_timestamp + timestamp_step;
                    }                
                },
                _ => {}
            }

            match event {
                Event::Backtrace { id, .. } | Event::PartialBacktrace { id, .. } => {
                    backtrace_id = Some( id );
                },
                Event::DecodedBacktrace { .. } => {
                    backtrace_id = Some( last_decoded_backtrace_id );
                    last_decoded_backtrace_id += 1;
                },
                Event::Alloc { allocation: AllocBody { pointer, .. }, .. } => {
                    match live_allocations.get( &pointer ) {
                        Some( &last_counter ) if counter == last_counter => {
                            counter += 1;
                            continue; // << added this to remove leaks as a WA
                        },
                        _ => {
                            counter += 1;
                            continue;
                        }
                    }
                },
                Event::Realloc { .. } => {
                    unreachable!();
                },
                Event::Free { .. } => {
                    unreachable!();
                },
                _ => {}
            }

            event.write_to_stream( &mut ofp )?;

            if let Some( id ) = backtrace_id {
                if let Some( stats ) = stats_by_backtrace.remove( &id ) {
                    let event = Event::GroupStatistics {
                        backtrace: id,
                        first_allocation: stats.first_allocation,
                        last_allocation: stats.last_allocation,
                        free_count: stats.free_count,
                        free_size: stats.free_size,
                        min_size: stats.min_size,
                        max_size: stats.max_size
                    };
                    event.write_to_stream( &mut ofp )?;
                }
            }
        }

        // remove remaining bucket allocations
        for bt in &used_backtraces {
            let event = Event::Free { 
                timestamp: timestamp_max,
                pointer: *bt,
                backtrace: *bt,
                thread: 1234
            };
            event.write_to_stream( &mut ofp )?;
        }
    }

    ofp.flush()?;
    mem::drop( ofp );

    assert!( stats_by_backtrace.is_empty() );
    let _ = fs::remove_file( tmpfile_path );

    Ok(())
}
