use crate::utils::HashMap;
use std::ops::Range;
use std::io::{Read, Write};
use std::borrow::Cow;
use common::event::{
    RegionFlags,
    Event
};
use common::speedy::Writable;
use crate::timestamp::Timestamp;
use crate::processing_thread::BacktraceCache;
use crate::unwind::Backtrace;

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum MapKind {
    Mmap = 0,
    Jemalloc = 1,
    Glibc = 2
}

impl MapKind {
    fn as_str( self ) -> &'static str {
        match self {
            MapKind::Mmap => "mmap",
            MapKind::Jemalloc => "jemalloc",
            MapKind::Glibc => "glibc"
        }
    }
}

#[derive(Copy, Clone)]
#[repr(transparent)]
struct CompactName( u64 );

impl CompactName {
    fn new( id: u64, kind: MapKind ) -> Self {
        Self( (id << 2) | (kind as u8 as u64) )
    }

    fn id( self ) -> u64 {
        self.0 >> 2
    }

    fn kind( self ) -> MapKind {
        unsafe { std::mem::transmute( (self.0 & 0b11) as u8 ) }
    }
}

#[test]
fn test_compact_name() {
    assert_eq!( CompactName::new( 0x1234FF, MapKind::Mmap ).id(), 0x1234FF );
    assert_eq!( CompactName::new( 0x1234FF, MapKind::Mmap ).kind(), MapKind::Mmap );
    assert_eq!( CompactName::new( 0x1234FF, MapKind::Mmap ).kind().as_str(), "mmap" );
    assert_eq!( CompactName::new( 0x1234FF, MapKind::Jemalloc ).id(), 0x1234FF );
    assert_eq!( CompactName::new( 0x1234FF, MapKind::Jemalloc ).kind(), MapKind::Jemalloc );
    assert_eq!( CompactName::new( 0x1234FF, MapKind::Jemalloc ).kind().as_str(), "jemalloc" );
}

#[inline(always)]
pub fn construct_name( id: u64, kind: &str ) -> crate::utils::Buffer< 32 > {
    let mut buffer = crate::utils::Buffer::< 32 >::new();
    write!( &mut buffer, "{}::{}\0", kind, id ).unwrap();
    buffer
}

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

#[derive(Clone)]
pub struct MapSource {
    pub timestamp: Timestamp,
    pub backtrace: Backtrace,
    pub tid: u32
}

#[derive(Clone)]
struct MapBucket {
    id: u64,
    source: MapSource
}

struct Mmap {
    id: u64,
    address: u64,
    requested_address: u64,
    requested_length: u64,
    mmap_protection: u32,
    mmap_flags: u32,
    file_descriptor: u32,
    offset: u64,
    source: MapSource
}

pub struct MapsRegistry {
    emulated_vma_name_map: fast_range_map::RangeMap< CompactName >,

    mmap_by_address: fast_range_map::RangeMap< MapBucket >,
    munmap_by_address: fast_range_map::RangeMap< MapBucket >,
    mmaps: HashMap< u64, Mmap >,
}

impl MapsRegistry {
    pub const fn new() -> Self {
        MapsRegistry {
            emulated_vma_name_map: fast_range_map::RangeMap::new(),
            mmap_by_address: fast_range_map::RangeMap::new(),
            munmap_by_address: fast_range_map::RangeMap::new(),
            mmaps: crate::utils::empty_hashmap(),
        }
    }

    pub fn set_vma_name( &mut self, pointer: *mut libc::c_void, length: usize, id: u64, kind: MapKind ) {
        if crate::global::is_pr_set_vma_anon_name_supported() {
            let name = construct_name( id, kind.as_str() );

            unsafe {
                crate::syscall::pr_set_vma_anon_name( pointer, length, &name );
            }
        } else {
            self.set_vma_name_slow( pointer, length, CompactName::new( id, kind ) );
        }
    }

    fn set_vma_name_slow( &mut self, pointer: *const libc::c_void, length: usize, name: CompactName ) {
        self.emulated_vma_name_map.insert( pointer as u64..pointer as u64 + length as u64, name );
    }

    pub fn clear_vma_name( &mut self, pointer: *mut libc::c_void, length: usize ) {
        if !crate::global::is_pr_set_vma_anon_name_supported() {
            self.emulated_vma_name_map.remove( pointer as u64..pointer as u64 + length as u64 );
        }
    }

    pub fn on_mmap(
        &mut self,
        id: u64,
        range: Range< u64 >,
        source: MapSource,
        requested_address: u64,
        mmap_protection: u32,
        mmap_flags: u32,
        file_descriptor: u32,
        offset: u64
    ) {
        for (range_unmapped, original_bucket) in self.mmap_by_address.remove( range.clone() ) {
            // When called with MAP_FIXED the `mmap` can also act as an `munmap`.

            let bucket = MapBucket { id: original_bucket.id, source: source.clone() };

            // If there were already any unmaps in this range leave them alone. They were first, so they should take precendence.
            let existing_unmaps: smallvec::SmallVec< [Range< u64 >; 2] > = self.munmap_by_address.get_in_range( range_unmapped.clone() ).map( |(range, _)| range.clone() ).collect();

            // Insert an unmap everywhere which was *not* already unmapped.
            let mut start = range_unmapped.start;
            for existing_unmap in existing_unmaps {
                trace!( "On munmap through fixed mmap: {:016X}..{:016X}, old_id = {}, new_id = {}", start, existing_unmap.start, original_bucket.id, id );
                self.munmap_by_address.insert( start..existing_unmap.start, bucket.clone() );
                start = existing_unmap.end;
            }
            trace!( "On munmap through fixed mmap: {:016X}..{:016X}, old_id = {}, new_id = {}", start, range_unmapped.end, original_bucket.id, id );
            self.munmap_by_address.insert( start..range_unmapped.end, bucket.clone() );
        }

        let bucket = MapBucket {
            id,
            source: source.clone()
        };

        trace!( "On mmap: 0x{:016X}..0x{:016X}, id = {}", range.start, range.end, id );
        self.mmap_by_address.insert( range.clone(), bucket );
        self.mmaps.insert( id, Mmap {
            id,
            address: range.start,
            requested_address,
            requested_length: range.end - range.start,
            mmap_protection,
            mmap_flags,
            file_descriptor,
            offset,
            source
        });
    }

    pub fn on_munmap( &mut self, range: Range< u64 >, source: MapSource ) {
        trace!( "On mummap: 0x{:016X}..0x{:016X}", range.start, range.end );

        if !crate::global::is_pr_set_vma_anon_name_supported() {
            self.emulated_vma_name_map.remove( range.clone() );
        }

        for (removed_range, bucket) in self.mmap_by_address.remove( range ) {
            trace!( "  Removed chunk: 0x{:016X}..0x{:016X}, id = {}", removed_range.start, removed_range.end, bucket.id );
            self.munmap_by_address.insert( removed_range, MapBucket { id: bucket.id, source: source.clone() } );
        }
    }
}

type RegionVec = smallvec::SmallVec< [Region; 1] >;
type SourcesVec = smallvec::SmallVec< [RegionRemovalSource; 1] >;

struct Map {
    regions: RegionVec
}

enum PendingEvent {
    Mmap {
        epoch: u64,
        mmap: Mmap
    },
    AddRegion {
        timestamp: Timestamp,
        epoch: u64,
        id: u64,
        info: RegionInfo,
        flags: RegionFlags,
        name: String,
    },
    UpdateUsage {
        timestamp: Timestamp,
        epoch: u64,
        id: u64,
        address: u64,
        length: u64,
        usage: RegionUsage
    },
    RemoveRegion {
        timestamp: Timestamp,
        epoch: u64,
        id: u64,
        address: u64,
        length: u64,
        sources: SourcesVec
    }
}

struct PendingMap {
    earliest_timestamp: Timestamp,
    events: smallvec::SmallVec< [PendingEvent; 1] >
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct RegionInfo {
    address: u64,
    length: u64,
    file_offset: u64,
    inode: u64,
    major: u32,
    minor: u32,
}

#[derive(PartialEq, Eq, Clone, Debug)]
struct RegionUsage {
    anonymous: u64,
    shared_clean: u64,
    shared_dirty: u64,
    private_clean: u64,
    private_dirty: u64,
    swap: u64,
}

#[derive(Debug)]
struct Region {
    info: RegionInfo,
    name: String,
    last_flags: RegionFlags,
    last_usage: RegionUsage,
}

struct RegionRemovalSource {
    address: u64,
    length: u64,
    source: MapSource
}

#[derive(Default)]
pub struct State {
    tmp_mmap_by_address: fast_range_map::RangeMap< MapBucket >,
    tmp_munmap_by_address: fast_range_map::RangeMap< MapBucket >,
    tmp_mmaps: HashMap< u64, Mmap >,
    tmp_buffer: Vec< u8 >,
    tmp_found_maps: HashMap< u64, RegionVec >,
    tmp_new_map_by_id: HashMap< u64, Map >,
    tmp_all_new_events: Vec< PendingEvent >,
    tmp_emulated_vma_name_map: fast_range_map::RangeMap< CompactName >,

    map_by_id: HashMap< u64, Map >,
    pending: HashMap< u64, PendingMap >,
    epoch: u64,
}

impl State {
    pub fn new_preallocated() -> Self {
        let mut state = Self::default();
        state.tmp_buffer.reserve( 64 * 1024 );
        state
    }

    fn clear_ephemeral( &mut self ) {
        self.tmp_mmap_by_address.clear();
        self.tmp_munmap_by_address.clear();
        self.tmp_mmaps.clear();
        self.tmp_buffer.clear();
        self.tmp_found_maps.clear();
        self.tmp_new_map_by_id.clear();
        self.tmp_all_new_events.clear();
        self.tmp_emulated_vma_name_map.clear();
    }
}

fn emit_mmap(
    mmap: Mmap,
    backtrace_cache: &mut BacktraceCache,
    serializer: &mut impl Write
) {
    let backtrace = crate::writers::write_backtrace( &mut *serializer, mmap.source.backtrace.clone(), backtrace_cache ).ok().unwrap_or( 0 );
    let source = common::event::RegionSource {
        timestamp: mmap.source.timestamp,
        backtrace,
        thread: mmap.source.tid
    };

    let _ = Event::MemoryMapEx {
        map_id: mmap.id,
        address: mmap.address,
        requested_address: mmap.requested_address,
        requested_length: mmap.requested_length,
        mmap_protection: mmap.mmap_protection,
        mmap_flags: mmap.mmap_flags,
        file_descriptor: mmap.file_descriptor,
        offset: mmap.offset,
        source
    }.write_to_stream( &mut *serializer );
}

fn emit_add_region(
    timestamp: Timestamp,
    map_id: u64,
    info: &RegionInfo,
    flags: RegionFlags,
    name: &str,
    serializer: &mut impl Write
) {
    let _ = Event::AddRegion {
        timestamp,
        map_id,
        address: info.address,
        length: info.length,
        file_offset: info.file_offset,
        inode: info.inode,
        major: info.major,
        minor: info.minor,
        flags,
        name: name.into()
    }.write_to_stream( &mut *serializer );
}

fn emit_usage(
    map_id: u64,
    address: u64,
    length: u64,
    timestamp: Timestamp,
    usage: RegionUsage,
    serializer: &mut impl Write
) {
    let _ = Event::UpdateRegionUsage {
        timestamp,
        map_id,
        address,
        length,
        anonymous: usage.anonymous,
        shared_clean: usage.shared_clean,
        shared_dirty: usage.shared_dirty,
        private_clean: usage.private_clean,
        private_dirty: usage.private_dirty,
        swap: usage.swap,
    }.write_to_stream( &mut *serializer );
}

fn emit_remove_region(
    timestamp: Timestamp,
    map_id: u64,
    address: u64,
    length: u64,
    sources: SourcesVec,
    backtrace_cache: &mut BacktraceCache,
    serializer: &mut impl Write
) {
    let sources_out: smallvec::SmallVec< [common::event::RegionTargetedSource; 1] > = sources.into_iter().map( |source| {
        let backtrace = crate::writers::write_backtrace( &mut *serializer, source.source.backtrace.clone(), backtrace_cache ).ok().unwrap_or( 0 );
        common::event::RegionTargetedSource {
            address: source.address,
            length: source.length,
            source: common::event::RegionSource {
                timestamp: source.source.timestamp,
                backtrace,
                thread: source.source.tid
            }
        }
    }).collect();

    let _ = Event::RemoveRegion {
        timestamp,
        map_id,
        address,
        length,
        sources: Cow::Borrowed( &sources_out )
    }.write_to_stream( &mut *serializer );
}

fn emit_events( backtrace_cache: &mut BacktraceCache, serializer: &mut impl Write, new_events: impl IntoIterator< Item = PendingEvent > ) {
    for event in new_events {
        match event {
            PendingEvent::Mmap { mmap, .. } => {
                emit_mmap(
                    mmap,
                    backtrace_cache,
                    serializer
                );
            },
            PendingEvent::AddRegion { timestamp, id, ref info, flags, name, .. } => {
                emit_add_region(
                    timestamp,
                    id,
                    info,
                    flags,
                    name.as_str(),
                    serializer
                );
            },
            PendingEvent::UpdateUsage { id, timestamp, address, length, usage, .. } => {
                emit_usage(
                    id,
                    address,
                    length,
                    timestamp,
                    usage,
                    serializer
                );
            },
            PendingEvent::RemoveRegion { timestamp, id, address, length, sources, .. } => {
                emit_remove_region(
                    timestamp,
                    id,
                    address,
                    length,
                    sources,
                    backtrace_cache,
                    serializer
                );
            }
        }
    }
}

fn generate_unmaps(
    tmp_munmap_by_address: &fast_range_map::RangeMap< MapBucket >,
    timestamp: Timestamp,
    epoch: u64,
    id: u64,
    old_region: &Region,
    output: &mut Vec< PendingEvent >
) {
    let address_start = old_region.info.address;
    let address_end = old_region.info.address + old_region.info.length;

    let mut sources = SourcesVec::new();

    // Let's try to find which calls resulted in its disappearance.
    for (unmap_range, unmap_bucket) in tmp_munmap_by_address.get_in_range( address_start..address_end ) {
        trace!( "Found a source for an unmap: 0x{:016X}, id = {}", unmap_range.start, id );

        sources.push( RegionRemovalSource {
            address: unmap_range.start,
            length: unmap_range.end - unmap_range.start,
            source: unmap_bucket.source.clone()
        });
    }

    output.push( PendingEvent::RemoveRegion {
        timestamp,
        epoch,
        id,
        address: old_region.info.address,
        length: old_region.info.length,
        sources
    });
}

pub fn update_smaps(
    state: &mut State,
    backtrace_cache: &mut BacktraceCache,
    serializer: &mut impl Write,
    force_emit: bool,
) {
    state.clear_ephemeral();
    state.epoch += 1;

    let timestamp;
    {
        let mut maps_registry = crate::global::MMAP_REGISTRY.lock().unwrap();

        maps_registry.mmap_by_address.clone_into( &mut state.tmp_mmap_by_address );
        std::mem::swap( &mut maps_registry.munmap_by_address, &mut state.tmp_munmap_by_address );
        maps_registry.munmap_by_address.clear();
        std::mem::swap( &mut maps_registry.mmaps, &mut state.tmp_mmaps );
        maps_registry.mmaps.clear();

        maps_registry.emulated_vma_name_map.clone_into( &mut state.tmp_emulated_vma_name_map );

        timestamp = crate::timestamp::get_timestamp();
        let mut fp = std::fs::File::open( "/proc/self/smaps" ).expect( "failed to open smaps" );
        fp.read_to_end( &mut state.tmp_buffer ).expect( "failed to read smaps" );

        std::mem::drop( maps_registry );
        std::mem::drop( fp );
    };

    let smaps = std::str::from_utf8( &state.tmp_buffer ).expect( "failed to parse smaps as UTF-8" ); // TODO: This is probably not always true.

    let region_info_to_id: HashMap< RegionInfo, (u64, usize) > =
        state.map_by_id.iter()
            .flat_map( |(id, map)| std::iter::once( id ).cycle().zip( map.regions.iter().enumerate() ) )
            .map( |(id, (region_index, region))| (region.info.clone(), (*id, region_index)) )
            .collect();

    let mut lines = smaps.trim().split( "\n" ).peekable();
    loop {
        let mut line = match lines.next() {
            Some( line ) => line,
            None => break
        };

        let address = u64::from_str_radix( get_until( &mut line, '-' ), 16 ).unwrap();
        let address_end = u64::from_str_radix( get_until( &mut line, ' ' ), 16 ).unwrap();
        let is_readable = if get_char( &mut line ).unwrap() == 'r' { RegionFlags::READABLE } else { RegionFlags::empty() };
        let is_writable = if get_char( &mut line ).unwrap() == 'w' { RegionFlags::WRITABLE } else { RegionFlags::empty() };
        let is_executable = if get_char( &mut line ).unwrap() == 'x' { RegionFlags::EXECUTABLE } else { RegionFlags::empty() };
        let is_shared = if get_char( &mut line ).unwrap() == 's' { RegionFlags::SHARED } else { RegionFlags::empty() };
        get_char( &mut line );

        let file_offset = u64::from_str_radix( get_until( &mut line, ' ' ), 16 ).unwrap();
        let major = u32::from_str_radix( get_until( &mut line, ':' ), 16 ).unwrap();
        let minor = u32::from_str_radix( get_until( &mut line, ' ' ), 16 ).unwrap();
        let inode: u64 = get_until( &mut line, ' ' ).parse().unwrap();
        skip_whitespace( &mut line );
        let mut name = Cow::Borrowed( line );
        let mut id: Option< u64 > = None;

        const BYTEHOUND_MEMFD_PREFIX: &str = "/memfd:bytehound::";

        // Try to extract the ID we've packed into the name.
        if crate::global::is_pr_set_vma_anon_name_supported() && name.starts_with( "[anon:" ) {
            // A name set with PR_SET_VMA_ANON_NAME.
            if let Some( index_1 ) = name.find( "::" ) {
                if let Some( length ) = name[ index_1 + 2.. ].find( "]" ) {
                    let index_2 = index_1 + 2 + length;
                    if index_2 + 1 == name.len() {
                        if let Ok( value ) = name[ index_1 + 2..index_2 ].parse() {
                            id = Some( value );
                            let mut cleaned_name = String::with_capacity( index_1 + 1 );
                            cleaned_name.push_str( &name[ ..index_1 ] );
                            cleaned_name.push_str( "]" );
                            name = Cow::Owned( cleaned_name );
                        }
                    }
                }
            }
        } else if name.starts_with( BYTEHOUND_MEMFD_PREFIX ) {
            let index_1 = BYTEHOUND_MEMFD_PREFIX.len();
            let index_2 = name[ index_1.. ].find( " " ).map( |index_2| index_1 + index_2 ).unwrap_or( name.len() );
            if let Ok( value ) = name[ index_1..index_2 ].parse() {
                id = Some( value );
                name = Cow::Borrowed( "[anon:bytehound]" );
            }
        } else if let Some( (_, compact_name) ) = state.tmp_emulated_vma_name_map.get( address ) {
            name = Cow::Owned( format!( "[anon:{}]", compact_name.kind().as_str() ) );
            id = Some( compact_name.id() );
        }

        let info = RegionInfo {
            address,
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

        let usage = RegionUsage {
            anonymous,
            shared_clean,
            shared_dirty,
            private_clean,
            private_dirty,
            swap,
        };

        if id.is_none() {
            // If we haven't managed to extract the ID from the name then try to match the region itself.
            //
            // This can happen if the name was changed by the application itself, or if it's just simply
            // a map which was mmaped outside of our control.
            if let Some( &(map_id, region_index) ) = region_info_to_id.get( &info ) {
                if state.map_by_id.get( &map_id ).unwrap().regions[ region_index ].name == name {
                    id = Some( map_id );
                }
            }

            // TODO: Handle maps which were split due to e.g. mprotect.
        }

        let id = id.unwrap_or_else( || crate::global::next_map_id() );
        let region = Region {
            info,
            name: name.into(),
            last_flags: flags,
            last_usage: usage,
        };

        state.tmp_found_maps.entry( id ).or_insert_with( RegionVec::new ).push( region );
    }

    for (id, new_regions) in state.tmp_found_maps.drain() {
        match state.map_by_id.remove( &id ) {
            Some( mut map ) => {
                // This is an existing map.
                let mut new_events = Vec::new();
                let mut merged_regions = RegionVec::new();
                for new_region in new_regions {
                    if let Some( old_region_index ) = map.regions.iter().position( |old_region| old_region.info == new_region.info && old_region.name == new_region.name ) {
                        // This is an existing region.
                        let mut old_region = map.regions.swap_remove( old_region_index );
                        if old_region.last_usage != new_region.last_usage {
                            new_events.push( PendingEvent::UpdateUsage {
                                epoch: state.epoch,
                                id,
                                timestamp,
                                address: new_region.info.address,
                                length: new_region.info.length,
                                usage: new_region.last_usage.clone()
                            });
                            old_region.last_usage = new_region.last_usage;
                        }
                        // TODO: Handle flag changes.
                        merged_regions.push( old_region );
                    } else {
                        // This is a brand new region.
                        trace!(
                            "Found new region for an existing map: 0x{:016X}, id = {}, source = {}",
                            new_region.info.address,
                            id,
                            state.tmp_mmap_by_address.get_value( new_region.info.address ).is_some()
                        );

                        new_events.push( PendingEvent::AddRegion {
                            timestamp,
                            epoch: state.epoch,
                            id,
                            info: new_region.info.clone(),
                            flags: new_region.last_flags,
                            name: new_region.name.clone(),
                        });
                        new_events.push( PendingEvent::UpdateUsage {
                            epoch: state.epoch,
                            id,
                            timestamp,
                            address: new_region.info.address,
                            length: new_region.info.length,
                            usage: new_region.last_usage.clone()
                        });
                        merged_regions.push( new_region );
                    }
                }

                for old_region in map.regions.drain( .. ) {
                    // This region doesn't exist anymore.

                    generate_unmaps(
                        &state.tmp_munmap_by_address,
                        timestamp,
                        state.epoch,
                        id,
                        &old_region,
                        &mut new_events
                    );
                }

                std::mem::swap( &mut map.regions, &mut merged_regions );

                if let Some( pending ) = state.pending.get_mut( &id ) {
                    // We haven't emitted this map yet.
                    if timestamp - pending.earliest_timestamp < CULLING_THRESHOLD && !force_emit {
                        // It still hasn't lived long enough to be emitted.
                        pending.events.extend( new_events.drain( .. ) );
                    } else {
                        // It has lived long enough; flush it.
                        state.tmp_all_new_events.extend( pending.events.drain( .. ) );
                        state.pending.remove( &id );
                    }
                }

                state.tmp_all_new_events.extend( new_events.drain( .. ) );
                state.tmp_new_map_by_id.insert( id, map );
            },
            None => {
                // This is a new map.
                let mut earliest_timestamp = timestamp;
                let mut events = smallvec::SmallVec::new();

                if let Some( mmap ) = state.tmp_mmaps.remove( &id ) {
                    earliest_timestamp = mmap.source.timestamp;
                    events.push( PendingEvent::Mmap {
                        epoch: state.epoch,
                        mmap
                    });
                }

                for region in &new_regions {
                    trace!( "Found new map: 0x{:016X}, id = {}, source = {}", region.info.address, id, state.tmp_mmap_by_address.get_value( region.info.address ).is_some() );
                    events.push( PendingEvent::AddRegion {
                        timestamp,
                        epoch: state.epoch,
                        id,
                        info: region.info.clone(),
                        flags: region.last_flags,
                        name: region.name.clone(),
                    });
                    events.push( PendingEvent::UpdateUsage {
                        epoch: state.epoch,
                        id,
                        timestamp,
                        address: region.info.address,
                        length: region.info.length,
                        usage: region.last_usage.clone()
                    });
                }

                state.pending.insert( id, PendingMap {
                    earliest_timestamp,
                    events
                });
                state.tmp_new_map_by_id.insert( id, Map { regions: new_regions } );
            }
        }
    }

    for (id, map) in state.map_by_id.drain() {
        // All of these maps were not picked up, which means they were all unmapped.
        if state.pending.remove( &id ).is_some() {
            // This map was not emitted.
            continue;
        }

        for region in map.regions {
            generate_unmaps(
                &state.tmp_munmap_by_address,
                timestamp,
                state.epoch,
                id,
                &region,
                &mut state.tmp_all_new_events
            );
        }
    }

    // Make sure any pending events are emitted in the proper order, and that the removals are prioritized.
    state.tmp_all_new_events.sort_unstable_by_key( |event| {
        match event {
            PendingEvent::Mmap { epoch, mmap: Mmap { id, address, .. }, .. } => (*epoch, 1, *id, *address),
            PendingEvent::AddRegion { epoch, id, info: RegionInfo { address, .. }, .. } => (*epoch, 2, *id, *address),
            PendingEvent::UpdateUsage { epoch, id, address, .. } => (*epoch, 3, *id, *address),
            PendingEvent::RemoveRegion { epoch, id, address, .. } => (*epoch, 0, *id, *address),
        }
    });

    emit_events( backtrace_cache, serializer, state.tmp_all_new_events.drain( .. ) );
    std::mem::swap( &mut state.map_by_id, &mut state.tmp_new_map_by_id );
}
