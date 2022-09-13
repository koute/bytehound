use crate::utils::HashMap;
use std::io::{Read, Write};
use std::borrow::Cow;
use common::event::{
    SMapFlags,
    Event
};
use common::speedy::Writable;
use crate::timestamp::Timestamp;
use crate::global::MapSource;
use crate::processing_thread::BacktraceCache;

const CULLING_THRESHOLD: Timestamp = Timestamp::from_secs( 1 );

fn get_until< 'a >( p: &mut &'a str, delimiter: char ) -> &'a str {
    let mut found = None;
    for (index, ch) in p.char_indices() {
        if ch == delimiter {
            found = Some( index );
            break;
        }
    }

    if let Some( index ) = found {
        let (before, after) = p.split_at( index );
        *p = &after[ delimiter.len_utf8().. ];
        before
    } else {
        let before = *p;
        *p = "";
        before
         }
     }

fn skip_whitespace( p: &mut &str ) {
    while let Some( ch ) = p.chars().next() {
        if ch == ' ' {
            *p = &p[ ch.len_utf8().. ];
        } else {
            break;
        }
    }
}

fn get_char( p: &mut &str ) -> Option< char > {
    let ch = p.chars().next()?;
    *p = &p[ ch.len_utf8().. ];
    Some( ch )
}

#[derive(PartialEq, Eq)]
struct SmapInfo {
    length: u64,
    file_offset: u64,
    inode: u64,
    major: u32,
    minor: u32,
}

#[derive(PartialEq, Eq, Clone)]
struct SmapUsage {
    anonymous: u64,
    shared_clean: u64,
    shared_dirty: u64,
    private_clean: u64,
    private_dirty: u64,
    swap: u64,
}

struct Pending {
    allocated_timestamp: Timestamp,
    flags: SMapFlags,
    source: Option< MapSource >,
    usage_history: smallvec::SmallVec< [(Timestamp, SmapUsage); 1] >,
}

struct Smap {
    info: SmapInfo,
    name: String,

    last_usage: SmapUsage,
    pending: Option< Pending >
}

impl Smap {
    fn is_on_hold( &self, timestamp: Timestamp ) -> bool {
        self.pending.as_ref().map( |pending| timestamp - pending.allocated_timestamp < CULLING_THRESHOLD ).unwrap_or( false )
    }
}

#[derive(Default)]
pub struct State {
    old_state: HashMap< u64, Smap >,
    new_state: HashMap< u64, Smap >,

    buffer: Vec< u8 >,
    source_maps: SourceMaps,
}

impl State {
    fn clear_buffers( &mut self ) {
        self.buffer.clear();
        self.source_maps.mmap_source_by_address.clear();
        self.source_maps.munmap_source_by_address.clear();
    }
}

#[derive(Default)]
struct SourceMaps {
    mmap_source_by_address: HashMap< usize, MapSource >,
    munmap_source_by_address: HashMap< usize, MapSource >,
}

impl SourceMaps {
    fn emit_unmap(
        &mut self,
        address: u64,
        timestamp: Timestamp,
        smap: &Smap,
        backtrace_cache: &mut BacktraceCache,
        serializer: &mut impl Write
    ) {
        let mut backtrace = 0;
        let mut thread = !0;
        if let Some( source ) = self.munmap_source_by_address.remove( &(address as usize) ) {
            backtrace = crate::writers::write_backtrace( &mut *serializer, source.backtrace.clone(), backtrace_cache ).ok().unwrap_or( 0 );
            thread = source.tid;
        }

        let _ = Event::RemoveMap {
            timestamp,
            address,
            length: smap.info.length,
            backtrace,
            thread
        }.write_to_stream( &mut *serializer );
    }
}

fn emit_map_if_pending(
    address: u64,
    smap: &mut Smap,
    backtrace_cache: &mut BacktraceCache,
    serializer: &mut impl Write
) {
    let mut pending = match smap.pending.take() {
        Some( pending ) => pending,
        None => return
    };

    let mut backtrace = 0;
    let mut thread = !0;
    if let Some( source ) = pending.source.take() {
        backtrace = crate::writers::write_backtrace( &mut *serializer, source.backtrace.clone(), backtrace_cache ).ok().unwrap_or( 0 );
        thread = source.tid;
    }

    let _ = Event::AddMap {
        timestamp: pending.allocated_timestamp,
        address,
        backtrace,
        thread,
        length: smap.info.length,
        file_offset: smap.info.file_offset,
        inode: smap.info.inode,
        major: smap.info.major,
        minor: smap.info.minor,
        flags: pending.flags,
        name: smap.name.as_str().into()
    }.write_to_stream( &mut *serializer );

    for (timestamp, usage) in pending.usage_history {
        emit_usage( address, timestamp, &usage, serializer );
    }
}

fn emit_usage(
    address: u64,
    timestamp: Timestamp,
    usage: &SmapUsage,
    serializer: &mut impl Write
) {
    let _ = Event::UpdateMapUsage {
        timestamp,
        address,
        anonymous: usage.anonymous,
        shared_clean: usage.shared_clean,
        shared_dirty: usage.shared_dirty,
        private_clean: usage.private_clean,
        private_dirty: usage.private_dirty,
        swap: usage.swap,
    }.write_to_stream( &mut *serializer );
}

pub fn update_smaps(
    timestamp: Timestamp,
    state: &mut State,
    backtrace_cache: &mut BacktraceCache,
    serializer: &mut impl Write,
    force_emit: bool,
) {
    state.clear_buffers();

    {
        let maps_registry = crate::global::MMAP_LOCK.lock().unwrap();

        maps_registry.mmap_source_by_address.clone_into( &mut state.source_maps.mmap_source_by_address );
        maps_registry.munmap_source_by_address.clone_into( &mut state.source_maps.munmap_source_by_address );

        let mut fp = std::fs::File::open( "/proc/self/smaps" ).expect( "failed to open smaps" );
        fp.read_to_end( &mut state.buffer ).expect( "failed to read smaps" );

        std::mem::drop( maps_registry );
        std::mem::drop( fp );
    };

    let smaps = std::str::from_utf8( &state.buffer ).expect( "failed to parse smaps as UTF-8" ); // TODO: This is probably not always true.

    let source_maps = &mut state.source_maps;
    let mut lines = smaps.trim().split( "\n" ).peekable();
    loop {
        let mut line = match lines.next() {
            Some( line ) => line,
            None => break
        };

        let address = u64::from_str_radix( get_until( &mut line, '-' ), 16 ).unwrap();
        let address_end = u64::from_str_radix( get_until( &mut line, ' ' ), 16 ).unwrap();
        let is_readable = if get_char( &mut line ).unwrap() == 'r' { SMapFlags::READABLE } else { SMapFlags::empty() };
        let is_writable = if get_char( &mut line ).unwrap() == 'w' { SMapFlags::WRITABLE } else { SMapFlags::empty() };
        let is_executable = if get_char( &mut line ).unwrap() == 'x' { SMapFlags::EXECUTABLE } else { SMapFlags::empty() };
        let is_shared = if get_char( &mut line ).unwrap() == 's' { SMapFlags::SHARED } else { SMapFlags::empty() };
        get_char( &mut line );

        let file_offset = u64::from_str_radix( get_until( &mut line, ' ' ), 16 ).unwrap();
        let major = u32::from_str_radix( get_until( &mut line, ':' ), 16 ).unwrap();
        let minor = u32::from_str_radix( get_until( &mut line, ' ' ), 16 ).unwrap();
        let inode: u64 = get_until( &mut line, ' ' ).parse().unwrap();
        skip_whitespace( &mut line );
        let name = Cow::Borrowed( line );

        let info = SmapInfo {
            length: address_end - address,
            file_offset,
            inode,
            major,
            minor
        };

        let flags = is_readable | is_writable | is_executable | is_shared;

        let mut rss = 0;
        let mut shared_clean = 0;
        let mut shared_dirty = 0;
        let mut private_clean = 0;
        let mut private_dirty = 0;
        let mut anonymous = 0;
        let mut swap = 0;
        while let Some( line ) = lines.peek() {
            let mut line = *line;
            let key = get_until( &mut line, ':' );
            if key.as_bytes().contains( &b' ' ) {
                break;
            }

            skip_whitespace( &mut line );
            let value = get_until( &mut line, ' ' );

            match key {
                "Rss" => rss = value.parse().unwrap(),
                "Shared_Clean" => shared_clean = value.parse().unwrap(),
                "Shared_Dirty" => shared_dirty = value.parse().unwrap(),
                "Private_Clean" => private_clean = value.parse().unwrap(),
                "Private_Dirty" => private_dirty = value.parse().unwrap(),
                "Anonymous" => anonymous = value.parse().unwrap(),
                "Swap" => swap = value.parse().unwrap(),
                _ => {}
            }

            lines.next();
        }

        debug_assert_eq!( rss, shared_clean + shared_dirty + private_clean + private_dirty );

        let usage = SmapUsage {
            anonymous,
            shared_clean,
            shared_dirty,
            private_clean,
            private_dirty,
            swap,
        };

        let smap =
            state.old_state.remove( &address )
            .and_then( |mut smap| {
                // A map already existed at this address when we previously ran.
                if smap.info != info || smap.name != name {
                    // It's a different map now.
                    if smap.pending.is_some() {
                        // The old map didn't live long enough; cull it.
                    } else {
                        // The old map *did* live long enough; synthesize an unmap for it.
                        source_maps.emit_unmap(
                            address,
                            timestamp,
                            &smap,
                            backtrace_cache,
                            serializer
                        )
                    }

                    return None;
                }

                // It's the same map.
                if usage != smap.last_usage {
                    // The map's usage changed.
                    smap.last_usage = usage.clone();
                    if let Some( ref mut pending ) = smap.pending {
                        // The map wasn't yet flushed; buffer the usage change.
                        pending.usage_history.push( (timestamp, usage.clone()) );
                    } else {
                        // The map was flushed, so we can just emit the usage change into the stream.
                        emit_usage(
                            address,
                            timestamp,
                            &usage,
                            serializer
                        );
                    }
                }

                Some( smap )
            });

        let mut smap = smap.unwrap_or_else( || {
            // The map did not exist at this address.
            Smap {
                info,
                name: name.into_owned(),
                last_usage: usage.clone(),
                pending: Some( Pending {
                    allocated_timestamp: timestamp,
                    flags,
                    source: source_maps.mmap_source_by_address.remove( &(address as usize) ),
                    usage_history: smallvec::smallvec![ (timestamp, usage) ],
                })
            }
        });

        if !smap.is_on_hold( timestamp ) || force_emit {
            // Emit the map if necessary.
            emit_map_if_pending(
                address,
                &mut smap,
                backtrace_cache,
                serializer
            );
        }

        state.new_state.insert( address, smap );
    }

    for (address, smap) in state.old_state.drain() {
        // All of these maps were not picked up, which means they were all deallocated.
        if smap.pending.is_some() {
            // This map was not emitted.
            continue;
        }

        source_maps.emit_unmap(
            address,
            timestamp,
            &smap,
            backtrace_cache,
            serializer
        )
    }

    std::mem::swap( &mut state.old_state, &mut state.new_state );
    state.clear_buffers();
}
