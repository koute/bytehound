#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

use std::collections::{BTreeSet, BTreeMap};
use std::fs::File;
use std::error::Error;
use std::sync::Arc;
use std::ops::Bound::{self, Unbounded};
use std::fmt::{self, Write};
use std::thread;
use std::io;
use std::borrow::Cow;
use std::cmp::{min, max, Ordering};
use std::path::{Path, PathBuf};
use std::iter::FusedIterator;

use actix_web::{
    App,
    Body,
    HttpRequest,
    HttpResponse,
    Result,
    server
};

use hashbrown::HashMap;

use actix_web::dev::Handler;
use actix_web::error::{ErrorNotFound, ErrorBadRequest, ErrorInternalServerError};
use actix_web::error::Error as ActixWebError;
use actix_web::http::Method;
use actix_web::middleware::cors::Cors;
use futures::Stream;
use serde::Serialize;
use itertools::Itertools;
use lru::LruCache;
use parking_lot::Mutex;

use cli_core::{
    Loader,
    Data,
    DataId,
    BacktraceId,
    Operation,
    Frame,
    Allocation,
    AllocationId,
    Tree,
    NodeId,
    FrameId,
    MalloptKind,
    VecVec,
    MmapOperation,
    MemoryMap,
    MemoryUnmap,
    CountAndSize,
    export_as_replay,
    export_as_heaptrack,
    export_as_flamegraph,
    export_as_flamegraph_pl,
    table_to_string
};

use common::Timestamp;

mod protocol;
mod streaming_channel;
mod byte_channel;
mod streaming_serializer;
mod filter;
#[macro_use]
mod rental;

use crate::byte_channel::byte_channel;
use crate::streaming_serializer::StreamingSerializer;
use crate::filter::{Filter, PrepareFilterError, prepare_filter, match_allocation};

struct AllocationGroups {
    allocations_by_backtrace: VecVec< BacktraceId, AllocationId >
}

impl AllocationGroups {
    fn new< 'a, T: IntoIterator< Item = (AllocationId, &'a Allocation) > >( iter: T ) -> Self {
        let mut grouped = HashMap::new();
        for (id, allocation) in iter {
            let allocations = grouped.entry( allocation.backtrace ).or_insert( Vec::new() );
            allocations.push( id );
        }

        let mut grouped: Vec< (BacktraceId, Vec< AllocationId >) > = grouped.into_iter().collect();
        grouped.sort_by_key( |&(backtrace_id, _)| backtrace_id );

        let mut allocations = VecVec::new();
        for (backtrace_id, allocation_ids) in grouped {
            allocations.insert( backtrace_id, allocation_ids );
        }

        let groups = AllocationGroups {
            allocations_by_backtrace: allocations
        };

        groups
    }

    fn len( &self ) -> usize {
        self.allocations_by_backtrace.len()
    }

    #[inline]
    fn iter( &self ) -> impl Iterator< Item = (BacktraceId, &[AllocationId]) > + ExactSizeIterator + FusedIterator {
        self.allocations_by_backtrace.iter()
            .map( |(&index, ids)| (index, ids) )
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AllocationGroupsKey {
    data_id: DataId,
    filter: protocol::AllocFilter,
    sort_by: protocol::AllocGroupsSortBy,
    order: protocol::Order
}

struct State {
    data: HashMap< DataId, Data >,
    data_ids: Vec< DataId >,
    allocation_group_cache: Mutex< LruCache< AllocationGroupsKey, Arc< AllocationGroups > > >
}

impl State {
    fn new() -> Self {
        State {
            data: HashMap::new(),
            data_ids: Vec::new(),
            allocation_group_cache: Mutex::new( LruCache::new( 4 ) )
        }
    }

    fn add_data( &mut self, data: Data ) {
        if self.data.contains_key( &data.id() ) {
            return;
        }

        self.data_ids.push( data.id() );
        self.data.insert( data.id(), data );
    }

    fn last_id( &self ) -> Option< DataId > {
        self.data_ids.last().cloned()
    }
}

type StateRef = Arc< State >;

fn query< 'a, T: serde::Deserialize< 'a >, S >( req: &'a HttpRequest< S > ) -> Result< T > {
    serde_urlencoded::from_str::<T>( req.query_string() )
        .map_err( |e| e.into() )
}

fn get_data_id( req: &HttpRequest< StateRef > ) -> Result< DataId > {
    let id = req.match_info().get( "id" ).unwrap();
    if id == "last" {
        return req.state().last_id().ok_or( ErrorNotFound( "data not found" ) );
    }

    let id: DataId = id.parse().map_err( |_| ErrorNotFound( "data not found" ) )?;
    if !req.state().data.contains_key( &id ) {
        return Err( ErrorNotFound( "data not found" ) );
    }
    Ok( id )
}

fn get_data( req: &HttpRequest< StateRef > ) -> Result< &Data > {
    let id = get_data_id( req )?;
    req.state().data.get( &id ).ok_or_else( || ErrorNotFound( "data not found" ) )
}

impl From< PrepareFilterError > for ActixWebError {
    fn from( error: PrepareFilterError ) -> Self {
        match error {
            PrepareFilterError::InvalidRegex( field, inner_err ) => {
                ErrorBadRequest( format!( "invalid '{}': {}", field, inner_err ) )
            }
        }
    }
}

fn async_data_handler< F: FnOnce( &Data, byte_channel::ByteSender ) + Send + 'static >( req: &HttpRequest< StateRef >, callback: F ) -> Result< Body > {
    let (tx, rx) = byte_channel();
    let rx = rx.map_err( |_| ErrorInternalServerError( "internal error" ) );
    let body = Body::Streaming( Box::new( rx ) );

    let data_id = get_data_id( &req )?;
    let state = req.state().clone();
    thread::spawn( move || {
        let data = match state.data.get( &data_id ) {
            Some( data ) => data,
            None => return
        };

        callback( data, tx );
    });

    Ok( body )
}

fn strip_template( input: &str ) -> String {
    let mut out = String::new();
    let mut buffered = String::new();
    let mut depth = 0;
    for ch in input.chars() {
        if ch == '<' {
            if out.ends_with( "operator" ) {
                out.push( ch );
                continue
            }

            if depth == 0 {
                buffered.clear();
                out.push( ch );
            } else {
                buffered.push( ch );
            }

            depth += 1;
            continue;
        }

        if depth > 0 {
            if ch == '>' {
                depth -= 1;
                if depth == 0 {
                    out.push_str( "..." );
                    out.push( ch );
                    buffered.clear();
                }

                continue;
            }
            buffered.push( ch );
            continue;
        }

        out.push( ch );
    }

    out.push_str( &buffered );
    out
}

fn get_frame< 'a >( data: &'a Data, format: &protocol::BacktraceFormat, frame: &Frame ) -> protocol::Frame< 'a > {
    let mut function = frame.function().map( |id| Cow::Borrowed( data.interner().resolve( id ).unwrap() ) );
    if format.strip_template_args.unwrap_or( false ) {
        function = function.map( |function| strip_template( &function ).into() );
    }

    protocol::Frame {
        address: frame.address().raw(),
        address_s: format!( "{:016X}", frame.address().raw() ),
        count: frame.count(),
        library: frame.library().map( |id| data.interner().resolve( id ).unwrap() ),
        function,
        raw_function: frame.raw_function().map( |id| data.interner().resolve( id ).unwrap() ),
        source: frame.source().map( |id| data.interner().resolve( id ).unwrap() ),
        line: frame.line(),
        column: frame.column(),
        is_inline: frame.is_inline()
    }
}

impl protocol::ResponseMetadata {
    fn new( data: &Data ) -> Self {
        protocol::ResponseMetadata {
            id: format!( "{}", data.id() ),
            executable: data.executable().to_owned(),
            architecture: data.architecture().to_owned(),
            final_allocated: data.total_allocated() - data.total_freed(),
            final_allocated_count: data.total_allocated_count() - data.total_freed_count(),
            runtime: (data.last_timestamp() - data.initial_timestamp()).into(),
            unique_backtrace_count: data.unique_backtrace_count() as u64,
            maximum_backtrace_depth: data.maximum_backtrace_depth(),
            timestamp: data.initial_timestamp().into()
        }
    }
}

fn handler_list( req: &HttpRequest< StateRef > ) -> HttpResponse {
    let list: Vec< _ > = req.state().data.values().map( |data| {
        protocol::ResponseMetadata::new( data )
    }).collect();

    HttpResponse::Ok().json( list )
}

fn get_fragmentation_timeline( data: &Data ) -> protocol::ResponseFragmentationTimeline {
    use std::ops::Range;

    #[derive(Clone)]
    struct Entry( Range< u64 > );
    impl PartialEq for Entry {
        #[inline(always)]
        fn eq( &self, lhs: &Entry ) -> bool {
            self.0.start == lhs.0.start
        }
    }
    impl Eq for Entry {}
    impl PartialOrd for Entry {
        #[inline(always)]
        fn partial_cmp( &self, rhs: &Entry ) -> Option< Ordering > {
            Some( self.cmp( rhs ) )
        }
    }
    impl Ord for Entry {
        #[inline(always)]
        fn cmp( &self, rhs: &Entry ) -> Ordering {
            self.0.start.cmp( &rhs.0.start )
        }
    }

    #[inline(always)]
    fn get_min( set: &BTreeSet< Entry > ) -> u64 {
        if set.is_empty() {
            return -1_i32 as u64;
        }

        let range = (Unbounded as Bound< Entry >, Unbounded);
        set.range( range ).next().unwrap().0.start
    }

    #[inline(always)]
    fn get_max( set: &BTreeSet< Entry > ) -> u64 {
        if set.is_empty() {
            return 0;
        }

        let range = (Unbounded as Bound< Entry >, Unbounded);
        set.range( range ).next_back().unwrap().0.end
    }

    #[inline(always)]
    fn is_matched( allocation: &Allocation ) -> bool {
        allocation.in_main_arena() && !allocation.is_mmaped()
    }

    let maximum_len = (data.last_timestamp().as_secs() - data.initial_timestamp().as_secs()) as usize;
    let mut xs = Vec::with_capacity( maximum_len );
    let mut x = (-1_i32) as u64;

    let mut current_used_address_space = 0;
    let mut fragmentation = Vec::with_capacity( maximum_len );
    let mut set: BTreeSet< Entry > = BTreeSet::new();
    let mut current_address_min = get_min( &set );
    let mut current_address_max = get_max( &set );

    for op in data.operations() {
        let timestamp = match op {
            Operation::Allocation { allocation, .. } => {
                if !is_matched( allocation ) {
                    continue;
                }

                let range = allocation.actual_range( &data );
                let entry = Entry( range.clone() );
                debug_assert!( !set.contains( &entry ) );
                set.insert( entry );
                current_used_address_space += range.end - range.start;

                if range.start < current_address_min {
                    current_address_min = range.start;
                }

                if range.end > current_address_max {
                    current_address_max = range.end;
                }

                allocation.timestamp
            },
            Operation::Deallocation { allocation, deallocation, .. } => {
                if !is_matched( allocation ) {
                    continue;
                }

                let range = allocation.actual_range( &data );
                let was_removed = set.remove( &Entry( range.clone() ) );
                assert!( was_removed );

                current_used_address_space -= range.end - range.start;

                if range.start == current_address_min {
                    current_address_min = get_min( &set );
                }
                if range.end == current_address_max {
                    current_address_max = get_max( &set );
                }

                deallocation.timestamp
            },
            Operation::Reallocation { new_allocation, old_allocation, .. } => {
                if !is_matched( new_allocation ) && !is_matched( old_allocation ) {
                    continue;
                }

                if is_matched( old_allocation ) {
                    let old_range = old_allocation.actual_range( &data );
                    let was_removed = set.remove( &Entry( old_range.clone() ) );
                    assert!( was_removed );

                    current_used_address_space -= old_range.end - old_range.start;
                }

                if is_matched( new_allocation ) {
                    let new_range = new_allocation.actual_range( &data );
                    let entry = Entry( new_range.clone() );
                    debug_assert!( !set.contains( &entry ) );
                    set.insert( entry );
                    current_used_address_space += new_range.end - new_range.start;
                }

                current_address_min = get_min( &set );
                current_address_max = get_max( &set );

                new_allocation.timestamp
            }
        };

        debug_assert_eq!( current_address_min, get_min( &set ) );
        debug_assert_eq!( current_address_max, get_max( &set ) );

        let timestamp = timestamp.as_secs();
        if timestamp != x {
            if x != (-1_i32 as u64) && x + 1 != timestamp {
                let last_fragmentation = fragmentation.last().cloned().unwrap();

                xs.push( x + 1 );
                fragmentation.push( last_fragmentation );

                if x + 2 != timestamp {
                    xs.push( timestamp - 1 );
                    fragmentation.push( last_fragmentation );
                }
            }

            x = timestamp;
            xs.push( x );
            fragmentation.push( 0 );
        }

        let range = if current_address_max == 0 {
            0
        } else {
            current_address_max - current_address_min
        };

        *fragmentation.last_mut().unwrap() = range - current_used_address_space;
    }

    protocol::ResponseFragmentationTimeline {
        xs,
        fragmentation
    }
}

fn handler_fragmentation_timeline( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let response = get_fragmentation_timeline( data );
    Ok( HttpResponse::Ok().json( response ) )
}

fn handler_timeline( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;

    let maximum_len = (data.last_timestamp().as_secs() - data.initial_timestamp().as_secs()) as usize;
    let mut xs = Vec::with_capacity( maximum_len );
    let mut size_delta = Vec::with_capacity( maximum_len );
    let mut count_delta = Vec::with_capacity( maximum_len );
    let mut allocated_size = Vec::with_capacity( maximum_len );
    let mut allocated_count = Vec::with_capacity( maximum_len );
    let mut leaked_size = Vec::with_capacity( maximum_len );
    let mut leaked_count = Vec::with_capacity( maximum_len );
    let mut x = (-1_i32) as u64;
    let mut current_size = 0;
    let mut current_count = 0;
    let mut current_leaked_size = 0;
    let mut current_leaked_count = 0;

    for op in data.operations() {
        let (timestamp, size_delta_v, count_delta_v) = match op {
            Operation::Allocation { allocation, .. } => {
                current_size += allocation.size;
                current_count += 1;

                if allocation.deallocation.is_none() {
                    current_leaked_size += allocation.size;
                    current_leaked_count += 1;
                }

                (allocation.timestamp, allocation.size as i64, 1)
            },
            Operation::Deallocation { allocation, deallocation, .. } => {
                current_size -= allocation.size;
                current_count -= 1;
                (deallocation.timestamp, allocation.size as i64 * -1, -1)
            },
            Operation::Reallocation { new_allocation, old_allocation, .. } => {
                current_size += new_allocation.size;
                current_size -= old_allocation.size;

                if new_allocation.deallocation.is_none() {
                    current_leaked_size += new_allocation.size;
                    current_leaked_count += 1;
                }

                (new_allocation.timestamp, new_allocation.size as i64 - old_allocation.size as i64, 0)
            }
        };

        let timestamp = timestamp.as_secs();
        if timestamp != x {
            if x != (-1_i32 as u64) && x + 1 != timestamp {
                let last_allocated_size = allocated_size.last().cloned().unwrap();
                let last_allocated_count = allocated_count.last().cloned().unwrap();
                let last_leaked_size = leaked_size.last().cloned().unwrap();
                let last_leaked_count = leaked_count.last().cloned().unwrap();

                xs.push( x + 1 );
                size_delta.push( 0 );
                count_delta.push( 0 );
                allocated_size.push( last_allocated_size );
                allocated_count.push( last_allocated_count );
                leaked_size.push( last_leaked_size );
                leaked_count.push( last_leaked_count );

                if x + 2 != timestamp {
                    xs.push( timestamp - 1 );
                    size_delta.push( 0 );
                    count_delta.push( 0 );
                    allocated_size.push( last_allocated_size );
                    allocated_count.push( last_allocated_count );
                    leaked_size.push( last_leaked_size );
                    leaked_count.push( last_leaked_count );
                }
            }

            x = timestamp;
            xs.push( x );
            size_delta.push( 0 );
            count_delta.push( 0 );
            allocated_size.push( 0 );
            allocated_count.push( 0 );
            leaked_size.push( 0 );
            leaked_count.push( 0 );
        }

        *allocated_size.last_mut().unwrap() = current_size;
        *allocated_count.last_mut().unwrap() = current_count;

        *leaked_size.last_mut().unwrap() = current_leaked_size;
        *leaked_count.last_mut().unwrap() = current_leaked_count;

        *size_delta.last_mut().unwrap() += size_delta_v;
        *count_delta.last_mut().unwrap() += count_delta_v;
    }

    let timeline = protocol::ResponseTimeline {
        xs,
        size_delta,
        count_delta,
        allocated_size,
        allocated_count,
        leaked_size,
        leaked_count
    };

    Ok( HttpResponse::Ok().json( timeline ) )
}

fn allocations_iter< 'a >( data: &'a Data, sort_by: protocol::AllocSortBy, order: protocol::Order, filter: &Filter ) -> impl DoubleEndedIterator< Item = (AllocationId, &'a Allocation) > {
    fn box_iter< 'a, I: 'a, U: 'a >( iter: I, order: protocol::Order ) -> Box< DoubleEndedIterator< Item = U > + 'a >
        where I: DoubleEndedIterator< Item = U >
    {
        match order {
            protocol::Order::Asc => Box::new( iter ),
            protocol::Order::Dsc => Box::new( iter.rev() )
        }
    }

    match sort_by {
        protocol::AllocSortBy::Timestamp => box_iter( data.alloc_sorted_by_timestamp( filter.timestamp_start_opt(), filter.timestamp_end_opt() ), order ),
        protocol::AllocSortBy::Address => box_iter( data.alloc_sorted_by_address( None, None ), order ),
        protocol::AllocSortBy::Size => box_iter( data.alloc_sorted_by_size( filter.size_min_opt(), filter.size_max_opt() ), order )
    }
}

fn timestamp_to_fraction( data: &Data, timestamp: Timestamp ) -> f32 {
    let relative = timestamp - data.initial_timestamp();
    let range = data.last_timestamp() - data.initial_timestamp();
    (relative.as_usecs() as f64 / range.as_usecs() as f64) as f32
}

fn get_allocations< 'a >( data: &'a Data, backtrace_format: protocol::BacktraceFormat, params: protocol::RequestAllocations, filter: Filter ) -> protocol::ResponseAllocations< impl Serialize + 'a > {
    let remaining = params.count.unwrap_or( -1_i32 as _ ) as usize;
    let skip = params.skip.unwrap_or( 0 ) as usize;
    let sort_by = params.sort_by.unwrap_or( protocol::AllocSortBy::Timestamp );
    let order = params.order.unwrap_or( protocol::Order::Asc );

    let total_count =
        allocations_iter( data, sort_by, order, &filter )
        .map( |(_, allocation)| allocation )
        .filter( |allocation| match_allocation( data, allocation, &filter ) ).count() as u64;

    let allocations = move || {
        let backtrace_format = backtrace_format.clone();
        let filter = filter.clone();

        allocations_iter( data, sort_by, order, &filter )
            .map( |(_, allocation)| allocation )
            .filter( move |allocation| match_allocation( data, allocation, &filter ) )
            .skip( skip )
            .take( remaining )
            .map( move |allocation| {
                let backtrace = data.get_backtrace( allocation.backtrace ).map( |(_, frame)| get_frame( data, &backtrace_format, frame ) ).collect();
                protocol::Allocation {
                    address: allocation.pointer,
                    address_s: format!( "{:016X}", allocation.pointer ),
                    timestamp: allocation.timestamp.into(),
                    timestamp_relative: (allocation.timestamp - data.initial_timestamp()).into(),
                    timestamp_relative_p: timestamp_to_fraction( data, allocation.timestamp ),
                    thread: allocation.thread,
                    size: allocation.size,
                    backtrace_id: allocation.backtrace.raw(),
                    deallocation: allocation.deallocation.as_ref().map( |deallocation| {
                        protocol::Deallocation {
                            timestamp: deallocation.timestamp.into(),
                            thread: deallocation.thread
                        }
                    }),
                    backtrace,
                    in_main_arena: !allocation.in_non_main_arena(),
                    is_mmaped: allocation.is_mmaped(),
                    extra_space: allocation.extra_usable_space
                }
            })
    };

    protocol::ResponseAllocations {
        allocations: StreamingSerializer::new( allocations ),
        total_count
    }
}

fn handler_allocations( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let params: protocol::RequestAllocations = query( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;
    let backtrace_format: protocol::BacktraceFormat = query( &req )?;

    let body = async_data_handler( &req, move |data, tx| {
        let response = get_allocations( data, backtrace_format, params, filter );
        let _ = serde_json::to_writer( tx, &response );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/json" ).body( body ) )
}

fn get_allocation_group_data< 'a, I >( data: &Data, iter: I ) -> protocol::AllocationGroupData
    where I: IntoIterator< Item = &'a Allocation >, <I as IntoIterator>::IntoIter: ExactSizeIterator
{
    let iter = iter.into_iter();
    assert_ne!( iter.len(), 0 );

    let mut size_sum = 0;
    let mut min_size = -1_i32 as _;
    let mut max_size = 0;
    let mut min_timestamp = Timestamp::max();
    let mut max_timestamp = Timestamp::min();
    let mut leaked_count = 0;
    let mut allocated_count = 0;
    for allocation in iter {
        let size = allocation.size;
        let timestamp = allocation.timestamp;
        size_sum += size;
        min_size = min( min_size, size );
        max_size = max( max_size, size );
        min_timestamp = min( min_timestamp, timestamp );
        max_timestamp = max( max_timestamp, timestamp );

        allocated_count += 1;
        if allocation.deallocation.is_none() {
            leaked_count += 1;
        }
    }

    protocol::AllocationGroupData {
        leaked_count,
        allocated_count,
        size: size_sum,
        min_size,
        max_size,
        min_timestamp: min_timestamp.into(),
        min_timestamp_relative: (min_timestamp - data.initial_timestamp()).into(),
        min_timestamp_relative_p: timestamp_to_fraction( data, min_timestamp ),
        max_timestamp: max_timestamp.into(),
        max_timestamp_relative: (max_timestamp - data.initial_timestamp()).into(),
        max_timestamp_relative_p: timestamp_to_fraction( data, max_timestamp ),
        interval: (max_timestamp - min_timestamp).into()
    }
}

fn get_global_group_data( data: &Data, backtrace_id: BacktraceId ) -> protocol::AllocationGroupData {
    let stats = data.get_group_statistics( backtrace_id );

    let leaked_count = stats.alloc_count - stats.free_count;
    let allocated_count = stats.alloc_count;
    let size_sum = stats.alloc_size;
    let min_size = stats.min_size;
    let max_size = stats.max_size;
    let min_timestamp = stats.first_allocation;
    let max_timestamp = stats.last_allocation;

    protocol::AllocationGroupData {
        leaked_count,
        allocated_count,
        size: size_sum,
        min_size,
        max_size,
        min_timestamp: min_timestamp.into(),
        min_timestamp_relative: (min_timestamp - data.initial_timestamp()).into(),
        min_timestamp_relative_p: timestamp_to_fraction( data, min_timestamp ),
        max_timestamp: max_timestamp.into(),
        max_timestamp_relative: (max_timestamp - data.initial_timestamp()).into(),
        max_timestamp_relative_p: timestamp_to_fraction( data, max_timestamp ),
        interval: (max_timestamp - min_timestamp).into()
    }
}

fn get_allocation_groups< 'a >(
    data: &'a Data,
    backtrace_format: protocol::BacktraceFormat,
    params: protocol::RequestAllocationGroups,
    allocation_groups: Arc< AllocationGroups >
) -> protocol::ResponseAllocationGroups< impl Serialize + 'a > {
    let remaining = params.count.unwrap_or( -1_i32 as _ ) as usize;
    let skip = params.skip.unwrap_or( 0 ) as usize;

    let total_count = allocation_groups.len();
    let factory = move || {
        let backtrace_format = backtrace_format.clone();
        let allocations = allocation_groups.clone();
        let iter = new_rental!( allocations, |allocations| allocations.iter() );
        iter
            .skip( skip )
            .take( remaining )
            .map( move |(backtrace_id, matched_allocation_ids)| {
                let all = get_global_group_data( data, backtrace_id );
                let only_matched = get_allocation_group_data( data, matched_allocation_ids.into_iter().map( |&allocation_id| data.get_allocation( allocation_id ) ) );
                let backtrace = data.get_backtrace( backtrace_id ).map( |(_, frame)| get_frame( data, &backtrace_format, frame ) ).collect();
                protocol::AllocationGroup {
                    all,
                    only_matched,
                    backtrace_id: backtrace_id.raw(),
                    backtrace
                }
            })
    };

    let response = protocol::ResponseAllocationGroups {
        allocations: StreamingSerializer::new( factory ),
        total_count: total_count as _
    };

    response
}

#[derive(PartialEq, Eq)]
struct Reverse< T >( T );

impl< T > PartialOrd for Reverse< T > where T: PartialOrd {
    #[inline]
    fn partial_cmp( &self, rhs: &Reverse< T > ) -> Option< Ordering > {
        self.0.partial_cmp( &rhs.0 ).map( Ordering::reverse )
    }
}

impl< T > Ord for Reverse< T > where T: Ord {
    #[inline]
    fn cmp( &self, rhs: &Reverse< T > ) -> Ordering {
        self.0.cmp( &rhs.0 ).reverse()
    }
}

fn handler_allocation_groups( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter_params: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter_params )?;
    let backtrace_format: protocol::BacktraceFormat = query( &req )?;
    let params: protocol::RequestAllocationGroups = query( &req )?;

    let key = AllocationGroupsKey {
        data_id: data.id(),
        filter: filter_params,
        sort_by: params.sort_by.unwrap_or( protocol::AllocGroupsSortBy::MinTimestamp ),
        order: params.order.unwrap_or( protocol::Order::Asc )
    };

    let groups = req.state().allocation_group_cache.lock().get( &key ).cloned();

    fn sort_by< T, F >( data: &Data, groups: &mut AllocationGroups, order: protocol::Order, is_global: bool, callback: F )
        where F: Fn( &protocol::AllocationGroupData ) -> T,
              T: Ord
    {
        groups.allocations_by_backtrace.sort_by_key( |(&backtrace_id, ids)| {
            let group_data = if is_global {
                get_global_group_data( data, backtrace_id )
            } else {
                let allocations = ids.iter().map( |&id| data.get_allocation( id ) );
                get_allocation_group_data( data, allocations )
            };
            callback( &group_data )
        });

        match order {
            protocol::Order::Asc => {},
            protocol::Order::Dsc => {
                groups.allocations_by_backtrace.reverse();
            }
        }
    }

    let allocation_groups;
    if let Some( groups ) = groups {
        allocation_groups = groups;
    } else {
        let iter =
            allocations_iter( data, Default::default(), Default::default(), &filter )
                .filter( move |(_, allocation)| match_allocation( data, allocation, &filter ) );

        let mut groups = AllocationGroups::new( iter );
        match key.sort_by {
            protocol::AllocGroupsSortBy::MinTimestamp => {
                sort_by( data, &mut groups, key.order, false, |group_data| group_data.min_timestamp.clone() );
            },
            protocol::AllocGroupsSortBy::MaxTimestamp => {
                sort_by( data, &mut groups, key.order, false, |group_data| group_data.max_timestamp.clone() );
            },
            protocol::AllocGroupsSortBy::Interval => {
                sort_by( data, &mut groups, key.order, false, |group_data| group_data.interval.clone() );
            },
            protocol::AllocGroupsSortBy::AllocatedCount => {
                sort_by( data, &mut groups, key.order, false, |group_data| group_data.allocated_count );
            },
            protocol::AllocGroupsSortBy::LeakedCount => {
                sort_by( data, &mut groups, key.order, false, |group_data| group_data.leaked_count );
            },
            protocol::AllocGroupsSortBy::Size => {
                sort_by( data, &mut groups, key.order, false, |group_data| group_data.size );
            },
            protocol::AllocGroupsSortBy::GlobalMinTimestamp => {
                sort_by( data, &mut groups, key.order, true, |group_data| group_data.min_timestamp.clone() );
            },
            protocol::AllocGroupsSortBy::GlobalMaxTimestamp => {
                sort_by( data, &mut groups, key.order, true, |group_data| group_data.max_timestamp.clone() );
            },
            protocol::AllocGroupsSortBy::GlobalInterval => {
                sort_by( data, &mut groups, key.order, true, |group_data| group_data.interval.clone() );
            },
            protocol::AllocGroupsSortBy::GlobalAllocatedCount => {
                sort_by( data, &mut groups, key.order, true, |group_data| group_data.allocated_count );
            },
            protocol::AllocGroupsSortBy::GlobalLeakedCount => {
                sort_by( data, &mut groups, key.order, true, |group_data| group_data.leaked_count );
            },
            protocol::AllocGroupsSortBy::GlobalSize => {
                sort_by( data, &mut groups, key.order, true, |group_data| group_data.size );
            }
        }

        allocation_groups = Arc::new( groups );
        req.state().allocation_group_cache.lock().put( key, allocation_groups.clone() );
    }

    let body = async_data_handler( &req, move |data, tx| {
        let response = get_allocation_groups( data, backtrace_format, params, allocation_groups );
        let _ = serde_json::to_writer( tx, &response );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/json" ).body( body ) )
}

fn handler_raw_allocations( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let iter = data.alloc_sorted_by_timestamp( None, None );

    let mut output = String::new();
    output.push_str( "[" );

    let mut is_first = true;
    for (_, allocation) in iter {
        if !is_first {
            output.push_str( "," );
        } else {
            is_first = false;
        }

        output.push_str( "{\"backtrace\":[" );
        let mut is_first = true;
        for (_, frame) in data.get_backtrace( allocation.backtrace ) {
            if !is_first {
                output.push_str( "," );
            } else {
                is_first = false;
            }

            let address = frame.address().raw();
            write!( output, "\"{:016X}\"", address ).unwrap();
        }
        output.push_str( "]}" );
    }

    output.push_str( "]" );
    Ok( HttpResponse::Ok().content_type( "application/json" ).body( output ) )
}

fn dump_node< T: fmt::Write, K: PartialEq + Clone, V, F: Fn( &mut T, &V ) -> fmt::Result >(
    tree: &Tree< K, V >,
    node_id: NodeId,
    output: &mut T,
    printer: &mut F
) -> fmt::Result {
    write!( output, "{{" )?;

    let node = tree.get_node( node_id );
    write!( output, "\"size\":{},", node.total_size )?;
    write!( output, "\"count\":{},", node.total_count )?;
    write!( output, "\"first\":{},", node.total_first_timestamp.as_secs() )?;
    write!( output, "\"last\":{},", node.total_last_timestamp.as_secs() )?;
    if let Some( value ) = node.value() {
        write!( output, "\"value\":" )?;
        printer( output, value )?;
        write!( output, "," )?;
    }

    write!( output, "\"children\":[" )?;
    for (index, &(_, child_id)) in tree.get_node( node_id ).children.iter().enumerate() {
        if index != 0 {
            write!( output, "," )?;
        }

        dump_node( tree, child_id, output, printer )?;
    }
    write!( output, "]" )?;

    write!( output, "}}" )?;
    Ok(())
}

fn handler_tree( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;
    let backtrace_format: protocol::BacktraceFormat = query( &req )?;

    let body = async_data_handler( &req, move |data, mut tx| {
        let mut tree: Tree< FrameId, &Frame > = Tree::new();
        for (allocation_id, allocation) in data.allocations_with_id() {
            if !match_allocation( data, allocation, &filter ) {
                continue;
            }

            tree.add_allocation( allocation, allocation_id, data.get_backtrace( allocation.backtrace ) );
        }

        dump_node( &tree, 0, &mut tx, &mut |output, frame| {
            let frame = get_frame( data, &backtrace_format, frame );
            serde_json::to_writer( output, &frame ).map_err( |_| fmt::Error )
        }).unwrap();
    })?;

    Ok( HttpResponse::Ok().content_type( "application/json" ).body( body ) )
}

fn handler_mmaps( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let backtrace_format: protocol::BacktraceFormat = query( &req )?;
    let body = async_data_handler( &req, move |data, tx| {
        let factory = || {
            data.mmap_operations().iter().map( |op| {
                match *op {
                    MmapOperation::Mmap( MemoryMap {
                        timestamp,
                        pointer,
                        length,
                        backtrace: backtrace_id,
                        requested_address,
                        mmap_protection,
                        mmap_flags,
                        file_descriptor,
                        thread,
                        offset
                    }) => {
                        let backtrace = data.get_backtrace( backtrace_id ).map( |(_, frame)| get_frame( data, &backtrace_format, frame ) ).collect();
                        protocol::MmapOperation::Mmap {
                            timestamp: timestamp.into(),
                            pointer,
                            pointer_s: format!( "{:016}", pointer ),
                            length,
                            backtrace,
                            backtrace_id: backtrace_id.raw(),
                            requested_address,
                            requested_address_s: format!( "{:016}", requested_address ),
                            is_readable: mmap_protection.is_readable(),
                            is_writable: mmap_protection.is_writable(),
                            is_executable: mmap_protection.is_executable(),
                            is_semaphore: mmap_protection.is_semaphore(),
                            grows_down: mmap_protection.grows_down(),
                            grows_up: mmap_protection.grows_up(),
                            is_shared: mmap_flags.is_shared(),
                            is_private: mmap_flags.is_private(),
                            is_fixed: mmap_flags.is_fixed(),
                            is_anonymous: mmap_flags.is_anonymous(),
                            is_uninitialized: mmap_flags.is_uninitialized(),
                            offset,
                            file_descriptor: file_descriptor as i32,
                            thread
                        }
                    },
                    MmapOperation::Munmap( MemoryUnmap {
                        timestamp,
                        pointer,
                        length,
                        backtrace: backtrace_id,
                        thread
                    }) => {
                        let backtrace = data.get_backtrace( backtrace_id ).map( |(_, frame)| get_frame( data, &backtrace_format, frame ) ).collect();
                        protocol::MmapOperation::Munmap {
                            timestamp: timestamp.into(),
                            pointer,
                            pointer_s: format!( "{:016}", pointer ),
                            length,
                            backtrace,
                            backtrace_id: backtrace_id.raw(),
                            thread
                        }
                    }
                }
            })
        };

        let response = protocol::ResponseMmaps {
            operations: StreamingSerializer::new( factory )
        };

        let _ = serde_json::to_writer( tx, &response );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/json" ).body( body ) )
}

fn handler_backtrace( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let backtrace_id: u32 = req.match_info().get( "backtrace_id" ).unwrap().parse().unwrap();
    let backtrace_id = BacktraceId::new( backtrace_id );
    let backtrace = data.get_backtrace( backtrace_id );
    let backtrace_format: protocol::BacktraceFormat = query( &req )?;

    let mut frames = Vec::new();
    for (_, frame) in backtrace {
        frames.push( get_frame( data, &backtrace_format, frame ) );
    }

    let response = protocol::ResponseBacktrace {
        frames
    };

    Ok( HttpResponse::Ok().json( response ) )
}

fn generate_regions< 'a, F: Fn( &Allocation ) -> bool + Clone + 'a >( data: &'a Data, filter: F ) -> impl Serialize + 'a {
    let main_heap_start = data.alloc_sorted_by_address( None, None )
        .map( |(_, allocation)| allocation )
        .filter( |allocation| !allocation.is_mmaped() && allocation.in_main_arena() )
        .map( |allocation| allocation.actual_range( data ).start )
        .next()
        .unwrap_or( 0 );

    let main_heap_end = data.alloc_sorted_by_address( None, None )
        .map( |(_, allocation)| allocation )
        .rev()
        .filter( |allocation| !allocation.is_mmaped() && allocation.in_main_arena() )
        .map( |allocation| allocation.actual_range( data ).end )
        .next()
        .unwrap_or( 0 );

    let regions = move || {
        let filter = filter.clone();
        data.alloc_sorted_by_address( None, None )
            .map( |(_, allocation)| allocation )
            .filter( move |allocation| filter( allocation ) )
            .map( move |allocation| allocation.actual_range( data ) )
            .coalesce( |mut range, next_range| {
                if next_range.start <= range.end {
                    range.end = next_range.end;
                    Ok( range )
                } else {
                    Err( (range, next_range) )
                }
            })
            .map( |range| [range.start, range.end] )
    };

    protocol::ResponseRegions {
        main_heap_start,
        main_heap_end,
        main_heap_start_s: format!( "{}", main_heap_start ),
        main_heap_end_s: format!( "{}", main_heap_end ),
        regions: StreamingSerializer::new( regions )
    }
}

fn handler_regions( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;

    let body = async_data_handler( &req, move |data, tx| {
        let response = generate_regions( data, |allocation| match_allocation( data, allocation, &filter ) );
        let _ = serde_json::to_writer( tx, &response );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/json" ).body( body ) )
}

fn handler_mallopts( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let backtrace_format: protocol::BacktraceFormat = query( &req )?;

    let response: Vec< _ > = data.mallopts().iter().map( |mallopt| {
        let mut backtrace = Vec::new();
        for (_, frame) in data.get_backtrace( mallopt.backtrace ) {
            backtrace.push( get_frame( data, &backtrace_format, frame ) );
        }

        protocol::Mallopt {
            timestamp: mallopt.timestamp.into(),
            thread: mallopt.thread,
            backtrace_id: mallopt.backtrace.raw(),
            backtrace,
            raw_param: mallopt.kind.raw(),
            param: match mallopt.kind {
                MalloptKind::TrimThreshold  => Some( "M_TRIM_THRESHOLD" ),
                MalloptKind::TopPad         => Some( "M_TOP_PAD" ),
                MalloptKind::MmapThreshold  => Some( "M_MMAP_THRESHOLD" ),
                MalloptKind::MmapMax        => Some( "M_MMAP_MAX" ),
                MalloptKind::CheckAction    => Some( "M_CHECK_ACTION" ),
                MalloptKind::Perturb        => Some( "M_PERTURB" ),
                MalloptKind::ArenaTest      => Some( "M_ARENA_TEXT" ),
                MalloptKind::ArenaMax       => Some( "M_ARENA_MAX" ),
                MalloptKind::Other( _ )     => None
            }.map( |value| value.into() ),
            value: mallopt.value,
            result: mallopt.result
        }
    }).collect();

    Ok( HttpResponse::Ok().json( response ) )
}

fn handler_export_flamegraph_pl( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;

    let body = async_data_handler( &req, move |data, tx| {
        let _ = export_as_flamegraph_pl( data, tx, |allocation| match_allocation( data, allocation, &filter ) );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/octet-stream" ).body( body ) )
}

fn handler_export_flamegraph( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;

    let body = async_data_handler( &req, move |data, tx| {
        let _ = export_as_flamegraph( data, tx, |allocation| match_allocation( data, allocation, &filter ) );
    })?;

    Ok( HttpResponse::Ok().content_type( "image/svg+xml" ).body( body ) )
}

fn handler_export_replay( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;

    let body = async_data_handler( &req, move |data, tx| {
        let _ = export_as_replay( data, tx, |allocation| match_allocation( data, allocation, &filter ) );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/octet-stream" ).body( body ) )
}

fn handler_export_heaptrack( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;

    let body = async_data_handler( &req, move |data, tx| {
        let _ = export_as_heaptrack( data, tx, |allocation| match_allocation( data, allocation, &filter ) );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/octet-stream" ).body( body ) )
}

fn handler_allocation_ascii_tree( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let data = get_data( &req )?;
    let filter: protocol::AllocFilter = query( &req )?;
    let filter = prepare_filter( data, &filter )?;

    let body = async_data_handler( &req, move |data, mut tx| {
        let tree = data.tree_by_source( |allocation| match_allocation( data, allocation, &filter ) );
        let table = data.dump_tree( &tree );
        let table = table_to_string( &table );
        let _ = writeln!( tx, "{}", table );
    })?;

    Ok( HttpResponse::Ok().content_type( "text/plain; charset=utf-8" ).body( body ) )
}

fn handler_collation_json< F >( req: &HttpRequest< StateRef >, callback: F ) -> Result< HttpResponse >
    where F: Fn( &Data ) -> BTreeMap< String, BTreeMap< u32, CountAndSize > > + Send + 'static
{
    use serde_json::json;

    let body = async_data_handler( &req, move |data, tx| {
        let constants = callback( &data );
        let mut total_count = 0;
        let mut total_size = 0;
        let per_file: BTreeMap< _, _ > = constants.into_iter().map( |(key, per_line)| {
            let mut whole_file_count = 0;
            let mut whole_file_size = 0;
            let per_line: BTreeMap< _, _ > = per_line.into_iter().map( |(line, entry)| {
                whole_file_count += entry.count;
                whole_file_size += entry.size;
                total_count += entry.count;
                total_size += entry.size;
                let entry = json!({
                    "count": entry.count,
                    "size": entry.size
                });
                (line, entry)
            }).collect();

            let value = json!({
                "count": whole_file_count,
                "size": whole_file_size,
                "per_line": per_line
            });

            (key, value)
        }).collect();

        let response = json!({
            "count": total_count,
            "size": total_size,
            "per_file": per_file
        });

        let _ = serde_json::to_writer( tx, &response );
    })?;

    Ok( HttpResponse::Ok().content_type( "application/json; charset=utf-8" ).body( body ) )
}

fn handler_dynamic_constants( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    handler_collation_json( req, |data| data.get_dynamic_constants() )
}

fn handler_dynamic_statics( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    handler_collation_json( req, |data| data.get_dynamic_statics() )
}

fn handler_dynamic_constants_ascii_tree( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let body = async_data_handler( &req, move |data, mut tx| {
        let table = data.get_dynamic_constants_ascii_tree();
        let _ = writeln!( tx, "{}", table );
    })?;

    Ok( HttpResponse::Ok().content_type( "text/plain; charset=utf-8" ).body( body ) )
}

fn handler_dynamic_statics_ascii_tree( req: &HttpRequest< StateRef > ) -> Result< HttpResponse > {
    let body = async_data_handler( &req, move |data, mut tx| {
        let table = data.get_dynamic_statics_ascii_tree();
        let _ = writeln!( tx, "{}", table );
    })?;

    Ok( HttpResponse::Ok().content_type( "text/plain; charset=utf-8" ).body( body ) )
}

struct StaticHandler( &'static str, &'static [u8] );
impl< T > Handler< T > for StaticHandler {
    type Result = Result< HttpResponse >;
    fn handle( &self, _: &HttpRequest< T > ) -> Self::Result {
        let mime = mime_guess::guess_mime_type( Path::new( self.0 ) );
        let mime = format!( "{}", mime );
        Ok( HttpResponse::Ok().content_type( mime ).body( self.1 ) )
    }
}

include!( concat!( env!( "OUT_DIR" ), "/webui_assets.rs" ) );

#[derive(Debug)]
pub enum ServerError {
    BindFailed( io::Error ),
    Other( io::Error )
}

impl fmt::Display for ServerError {
    fn fmt( &self, fmt: &mut fmt::Formatter ) -> fmt::Result {
        match *self {
            ServerError::BindFailed( ref error ) if error.kind() == io::ErrorKind::AddrInUse => {
                write!( fmt, "cannot server the server: address is already in use; you probably want to pick a different port with `--port`" )
            },
            ServerError::BindFailed( ref error ) => write!( fmt, "failed to start the server: {}", error ),
            ServerError::Other( ref error ) => write!( fmt, "{}", error )
        }
    }
}

impl From< io::Error > for ServerError {
    fn from( error: io::Error ) -> Self {
        ServerError::Other( error )
    }
}

impl Error for ServerError {}

pub fn main( inputs: Vec< PathBuf >, debug_symbols: Vec< PathBuf >, load_in_parallel: bool, interface: &str, port: u16 ) -> Result< (), ServerError > {
    let mut state = State::new();

    if !load_in_parallel {
        for filename in inputs {
            info!( "Trying to load {:?}...", filename );
            let fp = File::open( filename )?;
            let data = Loader::load_from_stream( fp, &debug_symbols )?;
            state.add_data( data );
        }
    } else {
        let handles: Vec< thread::JoinHandle< io::Result< Data > > > = inputs.iter().map( move |filename| {
            let filename = filename.clone();
            let debug_symbols = debug_symbols.clone();
            thread::spawn( move || {
                info!( "Trying to load {:?}...", filename );
                let fp = File::open( filename )?;
                let data = Loader::load_from_stream( fp, debug_symbols )?;
                Ok( data )
            })
        }).collect();


        for handle in handles {
            let data = handle.join().unwrap()?;
            state.add_data( data );
        }
    }

    for (key, bytes) in WEBUI_ASSETS {
        debug!( "Static asset: '{}', length = {}", key, bytes.len() );
    }

    let state = Arc::new( state );
    let sys = actix::System::new( "server" );
    server::new( move || {
        App::with_state( state.clone() )
            .configure( |app| {
                let mut app = Cors::for_app( app );
                app
                    .resource( "/list", |r| r.f( handler_list ) )
                    .resource( "/data/{id}/timeline", |r| r.method( Method::GET ).f( handler_timeline ) )
                    .resource( "/data/{id}/fragmentation_timeline", |r| r.method( Method::GET ).f( handler_fragmentation_timeline ) )
                    .resource( "/data/{id}/allocations", |r| r.method( Method::GET ).f( handler_allocations ) )
                    .resource( "/data/{id}/allocation_groups", |r| r.method( Method::GET ).f( handler_allocation_groups ) )
                    .resource( "/data/{id}/raw_allocations", |r| r.method( Method::GET ).f( handler_raw_allocations ) )
                    .resource( "/data/{id}/tree", |r| r.method( Method::GET ).f( handler_tree ) )
                    .resource( "/data/{id}/mmaps", |r| r.method( Method::GET ).f( handler_mmaps ) )
                    .resource( "/data/{id}/backtrace/{backtrace_id}", |r| r.method( Method::GET ).f( handler_backtrace ) )
                    .resource( "/data/{id}/regions", |r| r.method( Method::GET ).f( handler_regions ) )
                    .resource( "/data/{id}/mallopts", |r| r.method( Method::GET ).f( handler_mallopts ) )
                    .resource( "/data/{id}/export/flamegraph", |r| r.method( Method::GET ).f( handler_export_flamegraph ) )
                    .resource( "/data/{id}/export/flamegraph/{filename}", |r| r.method( Method::GET ).f( handler_export_flamegraph ) )
                    .resource( "/data/{id}/export/flamegraph.pl", |r| r.method( Method::GET ).f( handler_export_flamegraph_pl ) )
                    .resource( "/data/{id}/export/flamegraph.pl/{filename}", |r| r.method( Method::GET ).f( handler_export_flamegraph_pl ) )
                    .resource( "/data/{id}/export/heaptrack", |r| r.method( Method::GET ).f( handler_export_heaptrack ) )
                    .resource( "/data/{id}/export/heaptrack/{filename}", |r| r.method( Method::GET ).f( handler_export_heaptrack ) )
                    .resource( "/data/{id}/export/replay", |r| r.method( Method::GET ).f( handler_export_replay ) )
                    .resource( "/data/{id}/export/replay/{filename}", |r| r.method( Method::GET ).f( handler_export_replay ) )
                    .resource( "/data/{id}/allocation_ascii_tree", |r| r.method( Method::GET ).f( handler_allocation_ascii_tree ) )
                    .resource( "/data/{id}/dynamic_constants", |r| r.method( Method::GET ).f( handler_dynamic_constants ) )
                    .resource( "/data/{id}/dynamic_constants/{filename}", |r| r.method( Method::GET ).f( handler_dynamic_constants ) )
                    .resource( "/data/{id}/dynamic_constants_ascii_tree", |r| r.method( Method::GET ).f( handler_dynamic_constants_ascii_tree ) )
                    .resource( "/data/{id}/dynamic_constants_ascii_tree/{filename}", |r| r.method( Method::GET ).f( handler_dynamic_constants_ascii_tree ) )
                    .resource( "/data/{id}/dynamic_statics", |r| r.method( Method::GET ).f( handler_dynamic_statics ) )
                    .resource( "/data/{id}/dynamic_statics/{filename}", |r| r.method( Method::GET ).f( handler_dynamic_statics ) )
                    .resource( "/data/{id}/dynamic_statics_ascii_tree", |r| r.method( Method::GET ).f( handler_dynamic_statics_ascii_tree ) )
                    .resource( "/data/{id}/dynamic_statics_ascii_tree/{filename}", |r| r.method( Method::GET ).f( handler_dynamic_statics_ascii_tree ) );

                for (key, bytes) in WEBUI_ASSETS {
                    app.resource( &format!( "/{}", key ), move |r| r.method( Method::GET ).h( StaticHandler( key, bytes ) ) );
                    if *key == "index.html" {
                        app.resource( "/", move |r| r.method( Method::GET ).h( StaticHandler( key, bytes ) ) );
                    }
                }

                app.register()
            })
    }).bind( &format!( "{}:{}", interface, port ) ).map_err( |err| ServerError::BindFailed( err ) )?
        .shutdown_timeout( 1 )
        .start();

    let _ = sys.run();
    Ok(())
}
