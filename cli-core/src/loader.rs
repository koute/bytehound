use std::mem;
use std::cell::RefCell;
use std::ops::{Deref, Range};
use std::io::{self, Read};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Instant;
use std::ffi::OsStr;
use std::cmp;

use std::collections::hash_map;
use ahash::AHashMap as HashMap;
use ahash::AHashSet as HashSet;
use byteorder::{BigEndian, LittleEndian, ByteOrder};
use nwind::{arch, BinaryData, AddressSpace, IAddressSpace, DebugInfoIndex};
use nwind::proc_maps::Region;
use nwind::proc_maps::parse as parse_maps;
use rayon::prelude::*;

use common::event::{
    self,
    Event,
    HeaderBody,
    AllocBody,
    FramesInvalidated,
    HEADER_FLAG_IS_LITTLE_ENDIAN
};
use fast_range_map::RangeMap;

use crate::frame::Frame;
use crate::data::{
    Allocation,
    AllocationChain,
    AllocationFlags,
    AllocationId,
    BacktraceId,
    BacktraceStorageRef,
    CodePointer,
    DataPointer,
    Data,
    DataId,
    Deallocation,
    FrameId,
    GroupStatistics,
    Mallopt,
    OperationId,
    ThreadId,
    Timestamp,
    StringInterner,
    StringId,
    UsageDelta,
    Map,
    MapId,
    MapDeallocation,
    MapRegion,
    MapRegionDeallocation,
    MapRegionDeallocationSource,
    MapUsage,
    MapSource,
    RegionFlags,
};
use crate::vecvec::DenseVecVec;
use crate::reader::parse_events;

#[derive(Clone, PartialEq, Eq, Default, Debug, Hash)]
pub struct AddressMapping {
    pub declared_address: u64,
    pub actual_address: u64,
    pub file_offset: u64,
    pub size: u64
}

fn clean_symbol( input: Cow< str > ) -> Cow< str > {
    // TODO: Make this faster.
    input
        .replace( "> >", ">>" )
        .replace( "std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char>>", "std::string" )
        .into()
}

#[test]
fn test_clean_symbol() {
    let symbol = "google::protobuf::RepeatedPtrField<std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char> >::TypeHandler*::Type google::protobuf::internal::RepeatedPtrFieldBase::Add<google::protobuf::RepeatedPtrField<std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char> >::TypeHandler>()";
    assert_eq!(
        clean_symbol( symbol.into() ),
        "google::protobuf::RepeatedPtrField<std::string::TypeHandler*::Type google::protobuf::internal::RepeatedPtrFieldBase::Add<google::protobuf::RepeatedPtrField<std::string::TypeHandler>()"
    );
}

pub struct Loader {
    id: DataId,
    header: HeaderBody,
    interner: RefCell< StringInterner >,
    address_space: Box< dyn IAddressSpace >,
    address_space_needs_reloading: bool,
    debug_info_index: DebugInfoIndex,
    binaries: HashMap< String, Arc< BinaryData > >,
    pending_address_space_maps: Vec< Region >,
    address_space_maps: RangeMap< Region >,
    backtraces: Vec< BacktraceStorageRef >,
    backtraces_storage: Vec< FrameId >,
    backtrace_to_id: HashMap< Vec< u64 >, BacktraceId >,
    backtrace_remappings: HashMap< u64, BacktraceId >,
    group_stats: Vec< GroupStatistics >,
    operations: Vec< (Timestamp, OperationId) >,
    allocations: Vec< Allocation >,
    allocation_map: HashMap< (u64, u64), AllocationId >,
    allocation_range_map: RangeMap< AllocationId >,
    allocation_range_map_dirty: bool,
    allocations_by_backtrace: HashMap< BacktraceId, Vec< AllocationId > >,
    frames: Vec< Frame >,
    frame_to_id: HashMap< Frame, FrameId >,
    frames_by_address: HashMap< u64, Range< usize > >,
    shared_ptr_backtraces: HashSet< BacktraceId >,
    shared_ptr_allocations: HashMap< DataPointer, AllocationId >,
    total_allocated: u64,
    total_allocated_count: u64,
    total_freed: u64,
    total_freed_count: u64,
    frame_skip_ranges: Vec< Range< u64 > >,
    symbol_new_range: Range< u64 >,
    marker: u32,
    mallopts: Vec< Mallopt >,
    timestamp_to_wall_clock: u64,
    is_little_endian: bool,
    maximum_backtrace_depth: u32,
    previous_backtrace_on_thread: HashMap< u32, Vec< u64 > >,
    string_id_map: HashMap< u32, StringId >,
    last_timestamp: Timestamp,
    maps: Vec< Map >,
    map_index_map: HashMap< u64, MapId >,
    delta_list_for_map: HashMap< MapId, Vec< (Timestamp, UsageDelta) > >,
    last_usage_for_region: HashMap< (MapId, usize), MapUsage >,
    freshly_mmaped: HashSet< MapId >,
}

fn address_to_frame< F: FnMut( Frame ) >( address_space: &dyn IAddressSpace, interner: &mut StringInterner, address: u64, mut callback: F ) {
    address_space.decode_symbol_while( address, &mut |frame| {
        let mut output = Frame::new_unknown( CodePointer::new( address ) );
        if let Some( str ) = frame.library.take() {
            output.set_library( interner.get_or_intern( get_basename( &str ) ) );
        }
        if let Some( str ) = frame.demangled_name.take() {
            let str = clean_symbol( str );
            output.set_function( interner.get_or_intern( str ) );
        }
        if let Some( str ) = frame.name.take() {
            output.set_raw_function( interner.get_or_intern( str ) );
        }
        if let Some( str ) = frame.file.take() {
            output.set_source( interner.get_or_intern( str ) );
        }
        if let Some( value ) = frame.line.take() {
            output.set_line( value as _ );
        }
        if let Some( value ) = frame.column.take() {
            output.set_column( value as _ );
        }

        output.set_is_inline( frame.is_inline );

        callback( output );
        true
    });
}

trait PointerSize: Into< u64 > {
    fn read< E: ByteOrder >( slice: &[u8] ) -> Self;
}

impl PointerSize for u32 {
    #[inline]
    fn read< E: ByteOrder >( slice: &[u8] ) -> Self {
        E::read_u32( slice )
    }
}

impl PointerSize for u64 {
    #[inline]
    fn read< E: ByteOrder >( slice: &[u8] ) -> Self {
        E::read_u64( slice )
    }
}

fn get_basename( path: &str ) -> &str {
    if path.is_empty() {
        return path;
    }

    let path = if path.as_bytes().last().cloned().unwrap() == b'/' {
        &path[ 0..path.len() - 1 ]
    } else {
        path
    };

    &path[ path.rfind( "/" ).map( |index| index + 1 ).unwrap_or( 0 ).. ]
}

fn into_key( id: event::AllocationId, pointer: DataPointer ) -> (u64, u64) {
    if !id.is_invalid() && !id.is_untracked() {
        (id.thread, id.allocation)
    } else {
        (0, pointer)
    }
}

impl Loader {
    pub fn new( header: HeaderBody, debug_info_index: DebugInfoIndex ) -> Self {
        let address_space: Box< dyn IAddressSpace > = match &*header.arch {
            "arm" => Box::new( AddressSpace::< arch::arm::Arch >::new() ),
            "x86_64" => Box::new( AddressSpace::< arch::amd64::Arch >::new() ),
            "mips64" => Box::new( AddressSpace::< arch::mips64::Arch >::new() ),
            "aarch64" => Box::new( AddressSpace::< arch::aarch64::Arch >::new() ),
            _ => panic!( "Unknown architecture: {}", header.arch )
        };

        let flags = header.flags;
        let timestamp = header.timestamp;
        let wall_clock_secs = header.wall_clock_secs;
        let wall_clock_nsecs = header.wall_clock_nsecs;

        let mut loader = Loader {
            id: header.id,
            header,
            interner: RefCell::new( StringInterner::new() ),
            address_space,
            address_space_needs_reloading: true,
            debug_info_index,
            binaries: Default::default(),
            pending_address_space_maps: Default::default(),
            address_space_maps: RangeMap::new(),
            backtraces: Default::default(),
            backtraces_storage: Default::default(),
            backtrace_to_id: Default::default(),
            backtrace_remappings: Default::default(),
            group_stats: Default::default(),
            operations: Vec::with_capacity( 100000 ),
            allocations: Vec::with_capacity( 100000 ),
            allocation_map: Default::default(),
            allocation_range_map: RangeMap::new(),
            allocation_range_map_dirty: true,
            allocations_by_backtrace: Default::default(),
            frames: Default::default(),
            frame_to_id: Default::default(),
            frames_by_address: Default::default(),
            shared_ptr_backtraces: Default::default(),
            shared_ptr_allocations: Default::default(),
            total_allocated: 0,
            total_allocated_count: 0,
            total_freed: 0,
            total_freed_count: 0,
            frame_skip_ranges: Vec::with_capacity( 4 ),
            symbol_new_range: -1_i64 as u64..0,
            marker: 0,
            mallopts: Default::default(),
            timestamp_to_wall_clock: 0,
            is_little_endian: (flags & HEADER_FLAG_IS_LITTLE_ENDIAN) != 0,
            maximum_backtrace_depth: 0,
            previous_backtrace_on_thread: Default::default(),
            string_id_map: Default::default(),
            last_timestamp: Timestamp::min(),
            maps: Default::default(),
            map_index_map: Default::default(),
            delta_list_for_map: Default::default(),
            last_usage_for_region: Default::default(),
            freshly_mmaped: Default::default(),
        };

        loader.update_timestamp_to_wall_clock( timestamp, wall_clock_secs, wall_clock_nsecs );
        loader
    }

    fn update_timestamp_to_wall_clock( &mut self, timestamp: Timestamp, wall_clock_secs: u64, wall_clock_nsecs: u64 ) {
        self.timestamp_to_wall_clock = Timestamp::from_timespec( wall_clock_secs, wall_clock_nsecs ).as_usecs().wrapping_sub( timestamp.as_usecs() );
    }

    pub fn load_from_stream_without_debug_info< F: Read + Send + 'static >( fp: F ) -> Result< Data, io::Error > {
        use std::iter;

        let empty: iter::Empty< &OsStr > = iter::empty();
        Loader::load_from_stream( fp, empty )
    }

    pub fn load_from_stream< F: Read + Send + 'static, D: AsRef< OsStr >, I: IntoIterator< Item = D > >( fp: F, debug_symbols: I ) -> Result< Data, io::Error > {
        debug!( "Starting to load data..." );

        let start_timestamp = Instant::now();
        let (header, event_stream) = parse_events( fp )?;

        let mut debug_info_index = DebugInfoIndex::new();
        for path in debug_symbols {
            debug_info_index.add( path.as_ref() );
        }

        let mut loader = Loader::new( header, debug_info_index );

        for event in event_stream {
            let event = event?;
            loader.process( event );
        }

        let output = loader.finalize();
        let elapsed = start_timestamp.elapsed();
        info!( "Loaded data in {}s {:03}", elapsed.as_secs(), elapsed.subsec_millis() );
        Ok( output )
    }

    fn shift_timestamp( &self, timestamp: Timestamp ) -> Timestamp {
        Timestamp::from_usecs( timestamp.as_usecs().wrapping_add( self.timestamp_to_wall_clock ) )
    }

    fn parse_flags( &self, backtrace: BacktraceId, flags: u32 ) -> AllocationFlags {
        let mut allocation_flags = AllocationFlags::empty();
        if self.shared_ptr_backtraces.contains( &backtrace ) {
            allocation_flags |= AllocationFlags::IS_SHARED_PTR;
        }

        if flags & event::ALLOC_FLAG_PREV_IN_USE != 0 {
            allocation_flags |= AllocationFlags::IS_PREV_IN_USE;
        }

        if flags & event::ALLOC_FLAG_MMAPED != 0 {
            allocation_flags |= AllocationFlags::IS_MMAPED;
        }

        if flags & event::ALLOC_FLAG_NON_MAIN_ARENA != 0 {
            allocation_flags |= AllocationFlags::IN_NON_MAIN_ARENA;
        }

        if flags & event::ALLOC_FLAG_CALLOC != 0 {
            allocation_flags |= AllocationFlags::IS_CALLOC;
        }

        if flags & event::ALLOC_FLAG_JEMALLOC != 0 {
            allocation_flags |= AllocationFlags::IS_JEMALLOC;
        }

        if self.shared_ptr_backtraces.contains( &backtrace ) {
            allocation_flags |= AllocationFlags::IS_SHARED_PTR;
        }

        allocation_flags
    }

    fn handle_alloc(
        &mut self,
        id: event::AllocationId,
        timestamp: Timestamp,
        pointer: DataPointer,
        size: u64,
        backtrace: BacktraceId,
        thread: ThreadId,
        flags: u32,
        extra_usable_space: u32,
    ) {
        self.last_timestamp = std::cmp::max( self.last_timestamp, timestamp );

        let flags = self.parse_flags( backtrace, flags );
        let allocation_id = AllocationId::new( self.allocations.len() as _ );
        let allocation = Allocation {
            pointer,
            timestamp,
            size,
            thread,
            backtrace,
            deallocation: None,
            reallocation: None,
            reallocated_from: None,
            first_allocation_in_chain: None,
            position_in_chain: 0,
            flags,
            extra_usable_space,
            marker: self.marker
        };

        let key = into_key( id, pointer );
        let entry = self.allocation_map.entry( key );
        if let hash_map::Entry::Occupied( entry ) = entry {
            warn!( "Duplicate allocation of 0x{:016X}; old backtrace = {:?}, new backtrace = {:?}", pointer, self.allocations[ entry.get().raw() as usize ].backtrace, backtrace );
            return;
        }

        let group_stats = &mut self.group_stats[ allocation.backtrace.raw() as usize ];
        group_stats.first_allocation = cmp::min( group_stats.first_allocation, timestamp );
        group_stats.last_allocation = cmp::max( group_stats.last_allocation, timestamp );
        group_stats.min_size = cmp::min( group_stats.min_size, allocation.usable_size() );
        group_stats.max_size = cmp::max( group_stats.max_size, allocation.usable_size() );
        group_stats.alloc_count += 1;
        group_stats.alloc_size += allocation.usable_size();

        self.allocations.push( allocation );
        entry.or_insert( allocation_id );
        self.total_allocated += size;
        self.total_allocated_count += 1;

        let op = OperationId::new_allocation( allocation_id );
        self.operations.push( (timestamp, op) );

        if flags.contains( AllocationFlags::IS_SHARED_PTR ) {
            self.shared_ptr_allocations.insert( pointer, allocation_id );
        }

        self.allocation_range_map_dirty = true;
        self.allocations_by_backtrace.get_mut( &backtrace ).unwrap().push( allocation_id );
    }

    fn handle_free(
        &mut self,
        id: event::AllocationId,
        timestamp: Timestamp,
        pointer: DataPointer,
        backtrace: Option< BacktraceId >,
        thread: ThreadId
    ) {
        self.last_timestamp = std::cmp::max( self.last_timestamp, timestamp );

        let key = into_key( id, pointer );
        let allocation_id = match self.allocation_map.remove( &key ) {
            Some( id ) => id,
            None => {
                debug!( "Unknown deallocation of 0x{:016X} at backtrace = {:?}", pointer, backtrace );
                return;
            }
        };

        let allocation = &mut self.allocations[ allocation_id.raw() as usize ];
        allocation.deallocation = Some( Deallocation { timestamp, thread, backtrace } );
        self.total_freed += allocation.size;
        self.total_freed_count += 1;
        let group_stats = &mut self.group_stats[ allocation.backtrace.raw() as usize ];
        group_stats.free_count += 1;
        group_stats.free_size += allocation.usable_size();

        let op = OperationId::new_deallocation( allocation_id );
        self.operations.push( (timestamp, op) );

        if allocation.is_shared_ptr() {
            self.shared_ptr_allocations.remove( &allocation.pointer );
        }

        self.allocation_range_map_dirty = true;
    }

    fn handle_realloc(
        &mut self,
        id: event::AllocationId,
        timestamp: Timestamp,
        old_pointer: DataPointer,
        new_pointer: DataPointer,
        size: u64,
        backtrace: BacktraceId,
        thread: ThreadId,
        flags: u32,
        extra_usable_space: u32,
    ) {
        self.last_timestamp = std::cmp::max( self.last_timestamp, timestamp );

        let old_key = into_key( id, old_pointer );
        let allocation_id = match self.allocation_map.remove( &old_key ) {
            Some( id ) => id,
            None => return
        };

        let flags = self.parse_flags( backtrace, flags );
        let reallocation_id = AllocationId::new( self.allocations.len() as _ );
        {
            let allocation = &mut self.allocations[ allocation_id.raw() as usize ];
            assert!( !allocation.is_shared_ptr() );

            allocation.deallocation = Some( Deallocation { timestamp, thread, backtrace: Some( backtrace ) } );
            allocation.reallocation = Some( reallocation_id );
            self.total_freed += allocation.size;
            self.total_freed_count += 1;
            self.group_stats[ allocation.backtrace.raw() as usize ].free_count += 1;
            self.group_stats[ allocation.backtrace.raw() as usize ].free_size += allocation.usable_size();
        }

        let reallocation = Allocation {
            pointer: new_pointer,
            timestamp,
            size,
            thread,
            backtrace,
            deallocation: None,
            reallocation: None,
            reallocated_from: Some( allocation_id ),
            first_allocation_in_chain: None,
            position_in_chain: 0,
            flags,
            extra_usable_space,
            marker: self.marker
        };

        let new_key = into_key( id, new_pointer );
        let entry = self.allocation_map.entry( new_key );
        if let hash_map::Entry::Occupied( entry ) = entry {
            warn!( "Duplicate allocation (during realloc) of 0x{:016X}; old backtrace = {:?}, new backtrace = {:?}", new_pointer, self.allocations[ entry.get().raw() as usize ].backtrace, backtrace );
            return;
        }

        let group_stats = &mut self.group_stats[ reallocation.backtrace.raw() as usize ];
        group_stats.first_allocation = cmp::min( group_stats.first_allocation, timestamp );
        group_stats.last_allocation = cmp::max( group_stats.last_allocation, timestamp );
        group_stats.min_size = cmp::min( group_stats.min_size, reallocation.usable_size() );
        group_stats.max_size = cmp::max( group_stats.max_size, reallocation.usable_size() );
        group_stats.alloc_count += 1;
        group_stats.alloc_size += reallocation.usable_size();

        self.allocations.push( reallocation );
        entry.or_insert( reallocation_id );
        self.total_allocated += size;
        self.total_allocated_count += 1;

        let op = OperationId::new_reallocation( reallocation_id );
        self.operations.push( (timestamp, op) );

        self.allocation_range_map_dirty = true;
        self.allocations_by_backtrace.get_mut( &backtrace ).unwrap().push( reallocation_id );
    }

    pub(crate) fn interner( &mut self ) -> &mut StringInterner {
        self.interner.get_mut()
    }

    pub(crate) fn lookup_backtrace( &mut self, backtrace: u64 ) -> Option< BacktraceId > {
        let backtrace_id = self.backtrace_remappings.get( &backtrace ).cloned()?;
        self.allocations_by_backtrace.entry( backtrace_id ).or_insert( Vec::new() );
        Some( backtrace_id )
    }

    fn scan< P: PointerSize, B: ByteOrder >( &self, base_address: u64, data: &[u8] ) {
        assert_eq!( data.len() % mem::size_of::< P >(), 0 );
        for (index, subslice) in data.chunks_exact( mem::size_of::< P >() ).enumerate() {
            let value: u64 = P::read::< B >( subslice ).into();
            let allocation_id = match self.shared_ptr_allocations.get( &value ) {
                Some( &value ) => value,
                None => continue
            };

            let container_address = base_address + (mem::size_of::< P >() * index) as u64;


            if let Some( &container_allocation_id ) = self.allocation_range_map.get_value( container_address ) {
                let allocation: &Allocation = &self.allocations[ allocation_id.raw() as usize ];
                let container_allocation = &self.allocations[ container_allocation_id.raw() as usize ];
                assert!( !allocation.was_deallocated() );
                assert!( !container_allocation.was_deallocated() );

                trace!(
                    "Found an instance of shared pointer #{} (0x{:016X}) at 0x{:016X} (0x{:016X} + {}, allocation #{})",
                    allocation_id.raw(),
                    value,
                    container_address,
                    base_address,
                    mem::size_of::< P >() * index,
                    container_allocation_id.raw()
                );

                // TODO
            }
        }
    }

    fn reload_address_space( &mut self ) {
        if !self.address_space_needs_reloading {
            return;
        }

        self.address_space_needs_reloading = false;

        let mut maps = Vec::new();

        self.frame_skip_ranges.clear();
        for region in std::mem::take( &mut self.pending_address_space_maps ) {
            if region.name.contains( "libmemory_profiler" ) || region.name.contains( "libbytehound" ) {
                if self.frame_skip_ranges.last().map( |last_range| last_range.end == region.start ).unwrap_or( false ) {
                    let mut last_range = self.frame_skip_ranges.last_mut().unwrap();
                    last_range.end = region.end;
                } else {
                    self.frame_skip_ranges.push( region.start..region.end );
                }
            }

            maps.push( (region.start..region.end, region) );
        }

        for range in &self.frame_skip_ranges {
            debug!( "Skip range: 0x{:016X}-0x{:016X}", range.start, range.end );
        }

        self.address_space_maps = RangeMap::from_vec( maps );
        let binaries: Vec< _ > = self.binaries.values().cloned().collect();
        for binary_data in binaries {
            self.scan_for_symbols( &binary_data );
        }

        let binaries = &self.binaries;
        let debug_info_index = &mut self.debug_info_index;
        let regions: Vec< Region > = self.address_space_maps.values().cloned().collect();
        self.address_space.reload( regions, &mut |region, handle| {
            handle.should_load_frame_descriptions( false );

            let basename = get_basename( &region.name );
            let debug_binary_data = if let Some( binary_data ) = binaries.get( &region.name ).cloned() {
                let debug_binary_data = debug_info_index.get( &basename, binary_data.debuglink(), binary_data.build_id() );
                handle.set_binary( binary_data );
                debug_binary_data
            } else {
                debug_info_index.get( &basename, None, None )
            };

            if let Some( debug_binary_data ) = debug_binary_data {
                handle.set_debug_binary( debug_binary_data.clone() );
            }
        });
    }

    fn scan_for_symbols( &mut self, binary_data: &BinaryData ) {
        if self.maps.is_empty() {
            return;
        }

        ::nwind::Symbols::each_from_binary_data( &binary_data, |range, name| {
            if name == "_Znwm" {
                let region = self.address_space_maps.values().find( |region| {
                    region.name == binary_data.name()
                });

                if let Some( region ) = region {
                    self.symbol_new_range = region.start + range.start..region.start + range.end;
                }
            }
        });
    }

    fn handle_backtrace( &mut self, id: BacktraceId, potentially_call_to_new: bool ) {
        let (offset, length) = self.backtraces[ id.raw() as usize ];
        self.maximum_backtrace_depth = cmp::max( self.maximum_backtrace_depth, length as _ );

        if potentially_call_to_new {
            let interner = self.interner.get_mut();
            let frames = &self.frames;
            let mut iter = self.backtraces_storage[ offset as usize.. ].iter().rev().flat_map( |&id| frames[ id ].raw_function().and_then( |id| interner.resolve( id ) ) );
            if let Some( name ) = iter.next() {
                if name == "_ZNSt16_Sp_counted_baseILN9__gnu_cxx12_Lock_policyE2EEC4Ev" {
                    self.shared_ptr_backtraces.insert( id );
                }
            }
        }

        self.allocations_by_backtrace.entry( id ).or_insert( Vec::new() );

        assert_eq!( self.group_stats.len(), id.raw() as usize );
        self.group_stats.push( Default::default() );
    }

    fn add_backtrace< F >( &mut self, raw_id: u64, addresses: Cow< [u64] >, mut callback: F ) -> Option< BacktraceId > where F: FnMut( FrameId, bool ) {
        self.reload_address_space();

        let mut is_call_to_new = false;
        let mut to_skip = addresses.iter().take_while( |&&address| {
            let address = address - 1;
            for range in &self.frame_skip_ranges {
                if address >= range.start && address <= range.end {
                    return true;
                }
            }

            if address >= self.symbol_new_range.start && address < self.symbol_new_range.end {
                is_call_to_new = true;
                return true;
            }

            false
        }).count();

        if to_skip > 0 {
            to_skip -= 1;
        }

        if let Some( target_id ) = self.backtrace_to_id.get( &addresses[ to_skip.. ] ).cloned() {
            self.backtrace_remappings.insert( raw_id, target_id );
            return None;
        }

        let mut addresses = addresses.into_owned();
        addresses.drain( 0..to_skip );

        let backtrace_storage_offset = self.backtraces_storage.len();
        let backtrace_storage = &mut self.backtraces_storage;
        let frames = &mut self.frames;
        let frame_to_id = &mut self.frame_to_id;
        let mut interner = self.interner.get_mut();

        for (index, &address) in addresses.iter().enumerate() {
            let is_bytehound_tail = index == 0 && self.frame_skip_ranges.iter().any( |range| address >= range.start && address <= range.end );
            let address = if address > 0 { address - 1 } else { 0 };
            if let Some( range ) = self.frames_by_address.get( &address ).cloned() {
                for index in range {
                    let frame_id = backtrace_storage[ index ];
                    backtrace_storage.push( frame_id );
                    callback( frame_id, false );
                }
            } else {
                let offset = backtrace_storage.len();
                address_to_frame( &*self.address_space, &mut interner, address, |frame| {
                    let (frame_id, is_new) = if let Some( &frame_id ) = frame_to_id.get( &frame ) {
                        (frame_id, false)
                    } else {
                        let frame_id = frames.len();
                        frame_to_id.insert( frame.clone(), frame_id );
                        frames.push( frame );
                        (frame_id, true)
                    };

                    callback( frame_id, is_new );
                    backtrace_storage.push( frame_id );
                });

                if is_bytehound_tail && backtrace_storage.len() > offset {
                    backtrace_storage.drain( offset..backtrace_storage.len() - 1 );
                }

                self.frames_by_address.insert( address, offset..backtrace_storage.len() );
            }
        }

        let id = BacktraceId::new( self.backtraces.len() as _ );
        self.backtrace_remappings.insert( raw_id, id );

        let backtrace_length = backtrace_storage.len() - backtrace_storage_offset;
        let backtrace_storage_ref = (backtrace_storage_offset as _, backtrace_length as _);

        self.backtrace_to_id.insert( addresses, id );
        self.backtraces.push( backtrace_storage_ref );

        self.handle_backtrace( id, is_call_to_new );
        Some( id )
    }

    pub(crate) fn expand_partial_backtrace(
        previous_backtrace_on_thread: &mut HashMap< u32, Vec< u64 > >,
        thread: u32,
        frames_invalidated: FramesInvalidated,
        partial_addresses: impl ExactSizeIterator< Item = u64 >
    ) -> Vec< u64 > {
        match frames_invalidated {
            FramesInvalidated::All => {
                let mut addresses = Vec::new();
                mem::swap(
                    previous_backtrace_on_thread.entry( thread ).or_insert( Vec::new() ),
                    &mut addresses
                );

                addresses.clear();
                addresses.extend( partial_addresses );
                addresses
            },
            FramesInvalidated::Some( frames_invalidated ) => {
                let old_addresses = previous_backtrace_on_thread.entry( thread ).or_insert( Vec::new() );
                let new_iter = partial_addresses;
                let old_iter = old_addresses.iter().cloned().skip( frames_invalidated as usize );
                new_iter.chain( old_iter ).collect()
            }
        }
    }

    pub(crate) fn process_backtrace_event< F >( &mut self, event: Event, callback: F ) -> Option< BacktraceId > where F: FnMut( FrameId, bool ) {
        match event {
            Event::PartialBacktrace { id: raw_id, thread, frames_invalidated, addresses: partial_addresses } => {
                let addresses = Self::expand_partial_backtrace(
                    &mut self.previous_backtrace_on_thread,
                    thread,
                    frames_invalidated,
                    partial_addresses.iter().cloned()
                );
                let backtrace_id = self.add_backtrace( raw_id, addresses.as_slice().into(), callback );
                *self.previous_backtrace_on_thread.get_mut( &thread ).unwrap() = addresses;

                backtrace_id
            },
            Event::PartialBacktrace32 { id: raw_id, thread, frames_invalidated, addresses: partial_addresses } => {
                let addresses = Self::expand_partial_backtrace(
                    &mut self.previous_backtrace_on_thread,
                    thread,
                    frames_invalidated,
                    partial_addresses.iter().cloned().map( |value| value as u64 )
                );
                let backtrace_id = self.add_backtrace( raw_id, addresses.as_slice().into(), callback );
                *self.previous_backtrace_on_thread.get_mut( &thread ).unwrap() = addresses;

                backtrace_id
            },
            Event::Backtrace { id: raw_id, addresses } => {
                self.add_backtrace( raw_id, addresses, callback )
            },
            Event::Backtrace32 { id: raw_id, addresses } => {
                self.add_backtrace( raw_id, addresses.iter().map( |&p| p as u64 ).collect(), callback )
            },
            _ => {
                unreachable!();
            }
        }
    }

    pub(crate) fn get_frame( &self, id: FrameId ) -> &Frame {
        &self.frames[ id ]
    }

    pub fn process( &mut self, event: Event ) {
        match event {
            Event::Header( header ) => {
                assert_eq!( header.id, self.header.id );
                assert_eq!( header.initial_timestamp, self.header.initial_timestamp );
            },
            Event::File { ref path, ref contents, .. } | Event::File64 { ref path, ref contents, .. } if path == "/proc/self/maps" => {
                let contents = String::from_utf8_lossy( &contents );
                trace!( "/proc/self/maps:\n{}", contents );

                self.pending_address_space_maps = parse_maps( &contents );
                self.address_space_needs_reloading = true;
            },
            Event::MemoryMapEx { map_id, address, requested_length, mmap_protection, mmap_flags, source, requested_address: _, file_descriptor: _, offset: _ } => {
                let backtrace = self.lookup_backtrace( source.backtrace ).unwrap();
                let source = MapSource {
                    timestamp: self.shift_timestamp( source.timestamp ),
                    backtrace,
                    thread: source.thread
                };

                self.last_timestamp = std::cmp::max( self.last_timestamp, source.timestamp );

                let mut flags = RegionFlags::empty();
                if mmap_protection & 0x1 != 0 {
                    flags.insert( RegionFlags::READABLE );
                }
                if mmap_protection & 0x2 != 0 {
                    flags.insert( RegionFlags::WRITABLE );
                }
                if mmap_protection & 0x4 != 0 {
                    flags.insert( RegionFlags::EXECUTABLE );
                }
                if mmap_flags & 0x1 != 0 {
                    flags.insert( RegionFlags::SHARED );
                }

                let id = MapId( self.maps.len() as _ );
                self.map_index_map.insert( map_id, id );

                let map = Map {
                    id,
                    timestamp: source.timestamp,
                    source: Some( source ),
                    deallocation: None,
                    regions: Default::default(), // This will get populated later.
                    usage_history: Default::default(),
                    pointer: address,
                    size: requested_length,
                    flags,
                    name: Default::default(), // This will get populated later.
                    peak_rss: 0, // This will get populated later.
                };

                self.maps.push( map );
                self.freshly_mmaped.insert( id );
                self.delta_list_for_map.insert( id, vec![ (source.timestamp, UsageDelta {
                    address_space: requested_length as i64,
                    anonymous: 0,
                    shared_clean: 0,
                    shared_dirty: 0,
                    private_clean: 0,
                    private_dirty: 0,
                    swap: 0
                })]);
            },
            Event::AddRegion { timestamp, map_id, address, length, file_offset, inode, major, minor, flags, name } => {
                let mut timestamp = self.shift_timestamp( timestamp );
                self.last_timestamp = std::cmp::max( self.last_timestamp, timestamp );

                let map: &mut Map;
                let delta_list;
                if let Some( &existing_id ) = self.map_index_map.get( &map_id ) {
                    map = &mut self.maps[ existing_id.0 as usize ];
                    delta_list = self.delta_list_for_map.get_mut( &existing_id ).unwrap();

                    if self.freshly_mmaped.remove( &existing_id ) {
                        // The region matches what we've mmaped; correct the timestamp.
                        if map.pointer == address && map.size == length && map.flags == flags {
                            assert!( timestamp >= map.timestamp );
                            timestamp = map.timestamp;
                        }

                        // Revert the delta we've added when handling `MemoryMapEx`.
                        //
                        // If the regions did not actually change this will essentially be a no-op
                        // since we'll be adding the address space substracted here again during all
                        // of these `AddRegion`s.
                        delta_list.push( (timestamp, UsageDelta {
                            address_space: map.size as i64 * -1,
                            anonymous: 0,
                            shared_clean: 0,
                            shared_dirty: 0,
                            private_clean: 0,
                            private_dirty: 0,
                            swap: 0
                        }));

                        map.name = (&*name).into();
                    }
                } else {
                    let id = MapId( self.maps.len() as _ );
                    self.map_index_map.insert( map_id, id );
                    self.maps.push( Map {
                        id,
                        timestamp,
                        source: None,
                        deallocation: None,
                        regions: Default::default(),
                        usage_history: Default::default(),
                        peak_rss: 0,
                        pointer: address,
                        size: length,
                        flags,
                        name: (&*name).into()
                    });
                    map = self.maps.last_mut().unwrap();
                    delta_list = self.delta_list_for_map.entry( id ).or_insert_with( Vec::new );
                }

                let region = MapRegion {
                    timestamp,
                    pointer: address,
                    size: length,
                    flags,
                    file_offset,
                    inode,
                    major,
                    minor,
                    name: name.into(),
                    deallocation: None
                };

                let usage = MapUsage {
                    timestamp,
                    address_space: length,
                    anonymous: 0,
                    shared_clean: 0,
                    shared_dirty: 0,
                    private_clean: 0,
                    private_dirty: 0,
                    swap: 0
                };

                trace!( "Add region: id = {} ({}), address = 0x{:016X}, length = {}, source = {}", self.map_index_map.get( &map_id ).unwrap().raw(), map_id, address, length, map.source.is_some() );

                self.last_usage_for_region.insert( (map.id, map.regions.len()), usage );
                map.regions.push( region );
                delta_list.push( (timestamp, UsageDelta {
                    address_space: length as i64,
                    anonymous: 0,
                    shared_clean: 0,
                    shared_dirty: 0,
                    private_clean: 0,
                    private_dirty: 0,
                    swap: 0
                }));
            },
            Event::RemoveRegion { timestamp, map_id, address, length, sources } => {
                let timestamp = self.shift_timestamp( timestamp );
                self.last_timestamp = std::cmp::max( self.last_timestamp, timestamp );

                let id = match self.map_index_map.get( &map_id ) {
                    Some( smap ) => *smap,
                    None => {
                        warn!( "Remove map for map which was not found: id = {}, address = 0x{:016X}", map_id, address );
                        return;
                    }
                };

                let sources = sources.iter().map( |source| {
                    let backtrace = self.lookup_backtrace( source.source.backtrace ).unwrap();
                    MapRegionDeallocationSource {
                        address: source.address,
                        length: source.length,
                        source: MapSource {
                            timestamp: self.shift_timestamp( source.source.timestamp ),
                            backtrace,
                            thread: source.source.thread
                        }
                    }
                }).collect();

                let map = &mut self.maps[ id.0 as usize ];
                let (region_index, region) = match map.regions.iter_mut().enumerate().rev().find( |(_, region)| region.pointer == address && region.size == length ) {
                    Some( region ) => region,
                    None => {
                        warn!( "Remove map for region which was not found: id = {}, address = 0x{:016X}, length = {}", map_id, address, length );
                        return
                    }
                };

                trace!( "Remove region: id = {} ({}), address = 0x{:016X}, length = {}", id.0, map_id, address, length );

                if region.size != length {
                    warn!( "Length doesn't match for region at 0x{:016X} region removal", address );
                }

                region.deallocation = Some( MapRegionDeallocation {
                    timestamp,
                    sources
                });

                let last_usage = self.last_usage_for_region.remove( &(map.id, region_index) ).unwrap();
                let delta_list = self.delta_list_for_map.get_mut( &map.id ).unwrap();
                delta_list.push( (timestamp, UsageDelta {
                    address_space: -(region.size as i64),
                    anonymous: -(last_usage.anonymous as i64),
                    shared_clean: -(last_usage.shared_clean as i64),
                    shared_dirty: -(last_usage.shared_dirty as i64),
                    private_clean: -(last_usage.private_clean as i64),
                    private_dirty: -(last_usage.private_dirty as i64),
                    swap: -(last_usage.swap as i64)
                }));
            },
            Event::UpdateRegionUsage { timestamp, map_id, address, length, anonymous, shared_clean, shared_dirty, private_clean, private_dirty, swap } => {
                let timestamp = self.shift_timestamp( timestamp );
                self.last_timestamp = std::cmp::max( self.last_timestamp, timestamp );

                let id = match self.map_index_map.get( &map_id ) {
                    Some( smap ) => *smap,
                    None => {
                        warn!( "Update for map which was not found: id = {}, address = 0x{:016X}", map_id, address );
                        return;
                    }
                };

                let map = &mut self.maps[ id.0 as usize ];
                let (region_index, region) = match map.regions.iter_mut().enumerate().rev().find( |(_, region)| region.pointer == address ) {
                    Some( region_index ) => region_index,
                    None => {
                        warn!( "Update for a map for region which was not found: id = {}, address = 0x{:016X}", map_id, address );
                        return
                    }
                };

                if region.size != length {
                    warn!( "Length doesn't match for region at 0x{:016X} for region update", address );
                    return;
                }

                let usage = MapUsage {
                    timestamp,
                    address_space: region.size,
                    anonymous: anonymous * 1024,
                    shared_clean: shared_clean * 1024,
                    shared_dirty: shared_dirty * 1024,
                    private_clean: private_clean * 1024,
                    private_dirty: private_dirty * 1024,
                    swap: swap * 1024,
                };

                let last_usage = self.last_usage_for_region.get_mut( &(map.id, region_index) ).unwrap();
                let delta_list = self.delta_list_for_map.get_mut( &map.id ).unwrap();
                delta_list.push( (timestamp, UsageDelta {
                    address_space: 0,
                    anonymous: usage.anonymous as i64 - last_usage.anonymous as i64,
                    shared_clean: usage.shared_clean as i64 - last_usage.shared_clean as i64,
                    shared_dirty: usage.shared_dirty as i64 - last_usage.shared_dirty as i64,
                    private_clean: usage.private_clean as i64 - last_usage.private_clean as i64,
                    private_dirty: usage.private_dirty as i64 - last_usage.private_dirty as i64,
                    swap: usage.swap as i64 - last_usage.swap as i64
                }));
                *last_usage = usage;
            },
            Event::File { ref path, ref contents, .. } | Event::File64 { ref path, ref contents, .. } => {
                if !contents.starts_with( b"\x7FELF" ) {
                    return;
                }

                trace!( "File: {}", path );
                if let Ok( binary_data ) = BinaryData::load_from_owned_bytes( &path, contents.clone().into_owned() ) {
                    self.scan_for_symbols( &binary_data );
                    self.binaries.insert( path.deref().to_owned(), Arc::new( binary_data ) );
                }
            },
            event @ Event::PartialBacktrace { .. } |
            event @ Event::PartialBacktrace32 { .. } |
            event @ Event::Backtrace { .. } |
            event @ Event::Backtrace32 { .. } => {
                self.process_backtrace_event( event, |_, _| {} );
            },
            Event::String { id, string } => {
                let target_id = self.interner.get_mut().get_or_intern( string );
                self.string_id_map.insert( id, target_id );
            },
            Event::DecodedFrame { address, library, raw_function, function, source, line, column, is_inline } => {
                let mut frame = Frame::new_unknown( CodePointer::new( address ) );
                if library != 0xFFFFFFFF {
                    frame.set_library( *self.string_id_map.get( &library ).unwrap() );
                }
                if raw_function != 0xFFFFFFFF {
                    frame.set_raw_function( *self.string_id_map.get( &raw_function ).unwrap() );
                }
                if function != 0xFFFFFFFF {
                    frame.set_function( *self.string_id_map.get( &function ).unwrap() );
                }
                if source != 0xFFFFFFFF {
                    frame.set_source( *self.string_id_map.get( &source ).unwrap() );
                }
                if line != 0xFFFFFFFF {
                    frame.set_line( line );
                }
                if column != 0xFFFFFFFF {
                    frame.set_column( column );
                }

                frame.set_is_inline( is_inline );
                self.frames.push( frame );
            },
            Event::DecodedBacktrace { frames } => {
                let id = BacktraceId::new( self.backtraces.len() as _ );
                self.backtrace_remappings.insert( id.raw() as _, id );

                let backtrace_storage_offset = self.backtraces_storage.len();
                self.backtraces_storage.extend( frames.iter().cloned().map( |id| id as usize ) );
                let backtrace_length = self.backtraces_storage.len() - backtrace_storage_offset;
                let backtrace_storage_ref = (backtrace_storage_offset as _, backtrace_length as _);
                self.backtraces.push( backtrace_storage_ref );
                self.handle_backtrace( id, true );
            },
            Event::Alloc { timestamp, allocation: AllocBody { pointer, size, backtrace, thread, flags, extra_usable_space, preceding_free_space: _ } } => {
                let timestamp = self.shift_timestamp( timestamp );
                let backtrace = self.lookup_backtrace( backtrace ).unwrap();
                self.handle_alloc( event::AllocationId::UNTRACKED, timestamp, pointer, size, backtrace, thread, flags, extra_usable_space );
            },
            Event::AllocEx { id, timestamp, allocation: AllocBody { pointer, size, backtrace, thread, flags, extra_usable_space, preceding_free_space: _ } } => {
                let timestamp = self.shift_timestamp( timestamp );
                let backtrace = self.lookup_backtrace( backtrace ).unwrap();
                self.handle_alloc( id, timestamp, pointer, size, backtrace, thread, flags, extra_usable_space );
            },
            Event::Realloc { timestamp, old_pointer, allocation: AllocBody { pointer, size, backtrace, thread, flags, extra_usable_space, preceding_free_space: _ } } => {
                let timestamp = self.shift_timestamp( timestamp );
                let backtrace = self.lookup_backtrace( backtrace ).unwrap();
                self.handle_realloc( event::AllocationId::UNTRACKED, timestamp, old_pointer, pointer, size, backtrace, thread, flags, extra_usable_space );
            },
            Event::ReallocEx { id, timestamp, old_pointer, allocation: AllocBody { pointer, size, backtrace, thread, flags, extra_usable_space, preceding_free_space: _ } } => {
                let timestamp = self.shift_timestamp( timestamp );
                let backtrace = self.lookup_backtrace( backtrace ).unwrap();
                self.handle_realloc( id, timestamp, old_pointer, pointer, size, backtrace, thread, flags, extra_usable_space );
            },
            Event::Free { timestamp, pointer, backtrace, thread } => {
                let timestamp = self.shift_timestamp( timestamp );
                let backtrace = self.lookup_backtrace( backtrace );
                self.handle_free( event::AllocationId::UNTRACKED, timestamp, pointer, backtrace, thread );
            },
            Event::FreeEx { id, timestamp, pointer, backtrace, thread } => {
                let timestamp = self.shift_timestamp( timestamp );
                let backtrace = self.lookup_backtrace( backtrace );
                self.handle_free( id, timestamp, pointer, backtrace, thread );
            },
            Event::MemoryMap { .. } => {
                // Not used anymore.
            },
            Event::MemoryUnmap { .. } => {
                // Not used anymore.
            },
            Event::Mallopt { timestamp, backtrace, thread, param, value, result } => {
                let timestamp = self.shift_timestamp( timestamp );
                let backtrace = self.lookup_backtrace( backtrace ).unwrap();
                let kind = param.into();
                let mallopt = Mallopt {
                    timestamp, backtrace,
                    thread,
                    kind,
                    value,
                    result
                };
                self.mallopts.push( mallopt );
            },
            Event::Environ { .. } => {
                // TODO
            },
            Event::WallClock { timestamp, sec, nsec } => {
                self.update_timestamp_to_wall_clock( timestamp, sec, nsec );
            },
            Event::Marker { value } => {
                self.marker = value;
            },
            Event::MemoryDump { address, length, data } => {
                if true {
                    // TODO
                    return;
                }

                if self.allocation_range_map_dirty {
                    let mut allocations: Vec< (Range< u64 >, AllocationId) > = Vec::with_capacity( self.allocations.len() );
                    for (allocation_id, allocation) in self.allocations.iter().enumerate() {
                        let allocation_id = AllocationId::new( allocation_id as _ );
                        if allocation.was_deallocated() {
                            continue;
                        }

                        allocations.push( (allocation.pointer..allocation.pointer + allocation.size, allocation_id) );
                    }

                    let count = allocations.len();
                    self.allocation_range_map = RangeMap::from_vec( allocations );
                    self.allocation_range_map_dirty = false;
                    assert_eq!( count, self.allocation_range_map.len() );
                }

                let length = length as usize;
                assert_eq!( data.len(), length );
                match (self.header.pointer_size, self.is_little_endian) {
                    (4, false) => self.scan::< u32, BigEndian >( address, &data ),
                    (4, true)  => self.scan::< u32, LittleEndian >( address, &data ),
                    (8, false) => self.scan::< u64, BigEndian >( address, &data ),
                    (8, true)  => self.scan::< u64, LittleEndian >( address, &data ),
                    _ => unreachable!()
                }
            },
            Event::GroupStatistics { backtrace, first_allocation, last_allocation, free_count, free_size, min_size, max_size } => {
                let first_allocation = self.shift_timestamp( first_allocation );
                let last_allocation = self.shift_timestamp( last_allocation );
                let backtrace = self.lookup_backtrace( backtrace ).unwrap();
                let group_stats = &mut self.group_stats[ backtrace.raw() as usize ];
                group_stats.first_allocation = cmp::min( group_stats.first_allocation, first_allocation );
                group_stats.last_allocation = cmp::max( group_stats.last_allocation, last_allocation );
                group_stats.min_size = cmp::min( group_stats.min_size, min_size );
                group_stats.max_size = cmp::max( group_stats.max_size, max_size );
                group_stats.alloc_count += free_count;
                group_stats.alloc_size += free_size;
                group_stats.free_count += free_count;
                group_stats.free_size += free_size;
            }
        }
    }

    pub fn finalize( mut self ) -> Data {
        std::mem::take( &mut self.last_usage_for_region );

        let mut chains = HashMap::new();
        for index in 0..self.allocations.len() {
            let mut allocation = &self.allocations[ index ];
            if allocation.reallocation.is_some() || allocation.reallocated_from.is_none() {
                continue;
            }

            let last_allocation_id = AllocationId::new( index as _ );
            let mut first_allocation_id = last_allocation_id;
            let mut chain_length = 1;
            while let Some( previous_id ) = allocation.reallocated_from {
                first_allocation_id = previous_id;
                allocation = &self.allocations[ previous_id.raw() as usize ];
                chain_length += 1;
            }

            chains.insert( first_allocation_id, AllocationChain {
                first: first_allocation_id,
                last: last_allocation_id,
                length: chain_length
            });

            let mut current = first_allocation_id;
            for position in 0.. {
                let alloc = &mut self.allocations[ current.raw() as usize ];
                alloc.first_allocation_in_chain = Some( first_allocation_id );
                alloc.position_in_chain = position;
                if let Some( next_id ) = alloc.reallocation {
                    current = next_id;
                } else {
                    break;
                }
            }
        }

        let initial_timestamp = self.shift_timestamp( self.header.initial_timestamp );
        let indices: Vec< AllocationId > = (0..self.allocations.len()).into_iter().map( |id| AllocationId::new( id as _ ) ).collect();

        for (raw_backtrace_id, stats) in self.group_stats.iter().enumerate() {
            let (backtrace_offset, backtrace_len) = self.backtraces[ raw_backtrace_id ];
            for &frame_id in &self.backtraces_storage[ backtrace_offset as usize..(backtrace_offset as usize + backtrace_len as usize) ] {
                self.frames[ frame_id ].increment_count( stats.alloc_count );
            }
        }

        fn cmp_by_time( allocations: &[Allocation], a_id: AllocationId, b_id: AllocationId ) -> std::cmp::Ordering {
            let a_alloc = &allocations[ a_id.raw() as usize ];
            let b_alloc = &allocations[ b_id.raw() as usize ];
            a_alloc.timestamp.cmp( &b_alloc.timestamp ).then_with( ||
                a_id.raw().cmp( &b_id.raw() )
            )
        }

        let mut sorted_by_timestamp = indices.clone();
        let mut sorted_by_address = indices.clone();
        let mut sorted_by_size = indices;
        {
            let allocations = &self.allocations;
            sorted_by_timestamp.par_sort_by( |&a_id, &b_id| cmp_by_time( allocations, a_id, b_id ) );
            sorted_by_address.par_sort_by_key( |index| allocations[ index.raw() as usize ].pointer );
            sorted_by_size.par_sort_by_key( |index| allocations[ index.raw() as usize ].size );
        }

        self.operations.par_sort_by_key( |(timestamp, _)| *timestamp );
        let operations: Vec< _ > = self.operations.into_iter().map( |(_, op)| op ).collect();

        let mut current_total_usage_by_backtrace = Vec::new();
        current_total_usage_by_backtrace.resize( self.backtraces.len(), 0 );

        let mut current_total_max_size_by_backtrace = Vec::new();
        current_total_max_size_by_backtrace.resize( self.backtraces.len(), (0, initial_timestamp) );

        for &op in &operations {
            let allocation = &self.allocations[ op.id().raw() as usize ];
            let backtrace = allocation.backtrace;
            let mut current = current_total_usage_by_backtrace[ backtrace.raw() as usize ];
            if op.is_deallocation() {
                current -= allocation.usable_size() as isize;
            } else if op.is_allocation() {
                current += allocation.usable_size() as isize;
            } else if op.is_reallocation() {
                let old_allocation = &self.allocations[ allocation.reallocated_from.unwrap().raw() as usize ];
                current += allocation.usable_size() as isize;
                current -= old_allocation.usable_size() as isize;
            }

            if current > current_total_max_size_by_backtrace[ backtrace.raw() as usize ].0 {
                current_total_max_size_by_backtrace[ backtrace.raw() as usize ] = (current, allocation.timestamp);
            }

            current_total_usage_by_backtrace[ backtrace.raw() as usize ] = current;
        }

        self.allocations.shrink_to_fit();
        self.frames.shrink_to_fit();
        self.backtraces.shrink_to_fit();
        self.backtraces_storage.shrink_to_fit();
        self.mallopts.shrink_to_fit();
        self.group_stats.shrink_to_fit();
        self.maps.shrink_to_fit();

        for (index, (_, timestamp)) in current_total_max_size_by_backtrace.into_iter().enumerate() {
            self.group_stats[ index ].max_total_usage_first_seen_at = timestamp;
        }

        let mut allocations_by_backtrace = DenseVecVec::new();
        let mut index: Vec< _ > = self.allocations_by_backtrace.into_iter().collect();
        index.sort_by_key( |&(k, _)| k );

        let allocations = &self.allocations;
        for (backtrace_id, mut allocation_ids) in index {
            debug_assert!( allocation_ids.is_empty() || allocations[ allocation_ids[ 0 ].raw() as usize ].backtrace == backtrace_id );
            allocation_ids.sort_by( |&a_id, &b_id| cmp_by_time( allocations, a_id, b_id ) );
            let index = allocations_by_backtrace.push( allocation_ids );
            assert_eq!( index, backtrace_id.raw() as usize );
        }

        allocations_by_backtrace.shrink_to_fit();

        let delta_list_for_map = self.delta_list_for_map;

        self.maps.par_iter_mut().for_each( |map| {
            let raw_delta_list = delta_list_for_map.get( &map.id ).unwrap();
            let mut new_delta_list = raw_delta_list.clone();
            new_delta_list.sort_by_key( |(timestamp, _)| *timestamp );
            for index in 0..new_delta_list.len() - 1 {
                if new_delta_list[ index ].0 == new_delta_list[ index + 1 ].0 {
                    let delta = std::mem::take( &mut new_delta_list[ index ].1 );
                    new_delta_list[ index + 1 ].1 += delta;
                }
            }

            new_delta_list.retain( |(_, delta)| *delta != UsageDelta::default() );

            if map.regions.len() == 1 {
                map.deallocation = map.regions[ 0 ].deallocation.as_ref().map( |deallocation| {
                    MapDeallocation {
                        timestamp: deallocation.timestamp,
                        source: deallocation.sources.first().map( |source| source.source.clone() )
                    }
                });
            } else {
                if map.regions.iter().all( |region| region.deallocation.is_some() ) {
                    let deallocation = map.regions.iter().map( |region| region.deallocation.as_ref().unwrap() ).max_by_key( |deallocation| deallocation.timestamp ).unwrap();
                    map.deallocation = Some( MapDeallocation {
                        timestamp: deallocation.timestamp,
                        source: deallocation.sources.first().map( |source| source.source.clone() )
                    });
                }
            }

            let mut last_usage = MapUsage {
                timestamp: Timestamp::min(),
                address_space: 0,
                anonymous: 0,
                shared_clean: 0,
                shared_dirty: 0,
                private_clean: 0,
                private_dirty: 0,
                swap: 0,
            };

            let mut peak_rss = 0;
            let mut usage_history = Vec::with_capacity( new_delta_list.len() );
            for (timestamp, delta) in new_delta_list.into_iter() {
                let usage = MapUsage {
                    timestamp,
                    address_space: (last_usage.address_space as i64 + delta.address_space) as u64,
                    anonymous: (last_usage.anonymous as i64 + delta.anonymous) as u64,
                    shared_clean: (last_usage.shared_clean as i64 + delta.shared_clean) as u64,
                    shared_dirty: (last_usage.shared_dirty as i64 + delta.shared_dirty) as u64,
                    private_clean: (last_usage.private_clean as i64 + delta.private_clean) as u64,
                    private_dirty: (last_usage.private_dirty as i64 + delta.private_dirty) as u64,
                    swap: (last_usage.swap as i64 + delta.swap) as u64,
                };

                peak_rss = std::cmp::max( peak_rss, usage.rss() );
                last_usage = usage.clone();
                usage_history.push( usage );
            }

            map.usage_history = usage_history;
            map.peak_rss = peak_rss;
        });

        let last_timestamp = self.group_stats.iter().map( |stats| stats.last_allocation ).max().unwrap_or( initial_timestamp );
        let last_timestamp = std::cmp::max( self.last_timestamp, last_timestamp );
        Data {
            id: self.id,
            initial_timestamp,
            last_timestamp,
            executable: String::from_utf8_lossy( &self.header.executable ).into_owned(),
            cmdline: String::from_utf8_lossy( &self.header.cmdline ).into_owned(),
            architecture: self.header.arch,
            pointer_size: self.header.pointer_size as _,
            interner: self.interner.into_inner(),
            allocations: self.allocations,
            sorted_by_timestamp,
            sorted_by_address,
            sorted_by_size,
            operations,
            frames: self.frames,
            backtraces: self.backtraces,
            backtraces_storage: self.backtraces_storage,
            allocations_by_backtrace,
            total_allocated: self.total_allocated,
            total_allocated_count: self.total_allocated_count,
            total_freed: self.total_freed,
            total_freed_count: self.total_freed_count,
            mallopts: self.mallopts,
            maximum_backtrace_depth: self.maximum_backtrace_depth,
            group_stats: self.group_stats,
            chains,
            map_ids: (0..self.maps.len()).map( |id| MapId( id as u64 ) ).collect(),
            maps: self.maps,
        }
    }
}
