use std::sync::Arc;
use ahash::AHashMap as HashMap;
use ahash::AHashSet as HashSet;
use parking_lot::Mutex;

use regex::{self, Regex};

use cli_core::{
    Allocation,
    AllocationId,
    BacktraceId,
    Data,
    Timestamp,
    Compile,
    TryMatch,
    MapId,
    Map,
    EvalOutput,
};

use crate::protocol;

#[derive(Clone, Debug)]
pub struct GroupFilter {
    pub interval_min: Option< Timestamp >,
    pub interval_max: Option< Timestamp >,
    pub leaked_allocations_min: Option< protocol::NumberOrPercentage >,
    pub leaked_allocations_max: Option< protocol::NumberOrPercentage >,
    pub allocations_min: usize,
    pub allocations_max: usize
}

#[derive(Clone, Debug)]
pub struct BacktraceFilter {
    pub backtrace_depth_min: usize,
    pub backtrace_depth_max: usize,
    pub function_regex: Option< Regex >,
    pub source_regex: Option< Regex >,
    pub negative_function_regex: Option< Regex >,
    pub negative_source_regex: Option< Regex >,
}

impl From< crate::protocol::NumberOrPercentage > for cli_core::NumberOrFractionOfTotal {
    fn from( value: crate::protocol::NumberOrPercentage ) -> Self {
        match value {
            crate::protocol::NumberOrPercentage::Absolute( value ) => cli_core::NumberOrFractionOfTotal::Number( value as _ ),
            crate::protocol::NumberOrPercentage::Percent( value ) => cli_core::NumberOrFractionOfTotal::Fraction( value as f64 / 100.0 )
        }
    }
}

fn run_custom_filter( data: &Arc< Data >, custom_filter: &protocol::CustomFilter ) -> Result< Option< EvalOutput >, cli_core::script::EvalError > {
    if let Some( ref custom_filter ) = custom_filter.custom_filter {
        if custom_filter.is_empty() {
            return Ok( None );
        }

        let args = cli_core::script::EngineArgs {
            data: Some( data.clone() ),
            .. cli_core::script::EngineArgs::default()
        };

        let env = Arc::new( Mutex::new( cli_core::script::VirtualEnvironment::new() ) );
        let engine = cli_core::script::Engine::new( env.clone(), args );
        return Ok( engine.run( &custom_filter )? );
    }

    Ok( None )
}

fn run_custom_allocation_filter( data: &Arc< Data >, custom_filter: &protocol::CustomFilter ) -> Result< Option< Arc< HashSet< AllocationId > > >, cli_core::script::EvalError > {
    match run_custom_filter( data, custom_filter )? {
        None => {
            Ok( None )
        },
        Some( EvalOutput::AllocationList( mut list ) ) => {
            Ok( Some( Arc::new( list.allocation_ids().iter().copied().collect() ) ) )
        },
        Some( EvalOutput::MapList( .. ) ) => {
            Err( "expected 'AllocationList', got 'MapList'".into() )
        }
    }
}

fn run_custom_map_filter( data: &Arc< Data >, custom_filter: &protocol::CustomFilter ) -> Result< Option< Arc< HashSet< MapId > > >, cli_core::script::EvalError > {
    match run_custom_filter( data, custom_filter )? {
        None => {
            Ok( None )
        },
        Some( EvalOutput::AllocationList( .. ) ) => {
            Err( "expected 'MapList', got 'AllocationList'".into() )
        },
        Some( EvalOutput::MapList( mut list ) ) => {
            Ok( Some( Arc::new( list.map_ids().iter().copied().collect() ) ) )
        }
    }
}

#[derive(Clone)]
pub struct AllocationFilter {
    filter: cli_core::CompiledAllocationFilter,
    custom_filter: Option< Arc< HashSet< AllocationId > > >
}

impl AllocationFilter {
    pub fn try_match( &self, data: &Data, id: AllocationId, allocation: &Allocation ) -> bool {
        if let Some( ref custom_filter ) = self.custom_filter {
            if !custom_filter.contains( &id ) {
                return false;
            }
        }

        if !self.filter.try_match( data, allocation ) {
            return false;
        }

        true
    }
}

pub fn prepare_allocation_filter(
    data: &Arc< Data >,
    filter: &protocol::AllocFilter,
    custom_filter: &protocol::CustomFilter
) -> Result< AllocationFilter, PrepareFilterError > {
    let filter = prepare_raw_allocation_filter( data, filter )?.compile( data );
    let custom_filter = run_custom_allocation_filter( data, custom_filter ).map_err( |error| PrepareFilterError::InvalidCustomFilter( error.message ) )?;

    Ok( AllocationFilter { filter, custom_filter } )
}

pub fn prepare_raw_allocation_filter( data: &Data, filter: &protocol::AllocFilter ) -> Result< cli_core::AllocationFilter, PrepareFilterError > {
    use cli_core::Duration;

    let mut output = cli_core::RawAllocationFilter::default();

    output.common_filter.only_allocated_after_at_least = filter.from.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.common_filter.only_allocated_until_at_most = filter.to.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.common_filter.only_address_at_least = filter.address_min;
    output.common_filter.only_address_at_most = filter.address_max;
    output.common_filter.only_larger_or_equal = filter.size_min;
    output.common_filter.only_smaller_or_equal = filter.size_max;
    output.only_first_size_larger_or_equal = filter.first_size_min;
    output.only_first_size_smaller_or_equal = filter.first_size_max;
    output.only_last_size_larger_or_equal = filter.last_size_min;
    output.only_last_size_smaller_or_equal = filter.last_size_max;

    if let Some( only_alive_at ) = filter.alive_at.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) ) {
        output.common_filter.only_alive_at.push( only_alive_at );
    }
    if let Some( only_alive_at ) = filter.alive_at_2.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) ) {
        output.common_filter.only_alive_at.push( only_alive_at );
    }

    if let Some( from_map ) = filter.from_map {
        output.only_from_maps = Some( std::iter::once( MapId( from_map ) ).collect() );
    }

    output.common_filter.only_alive_for_at_least = filter.lifetime_min.map( |interval| Duration( interval.0 ) );
    output.common_filter.only_alive_for_at_most = filter.lifetime_max.map( |interval| Duration( interval.0 ) );

    output.backtrace_filter.only_backtrace_length_at_least = filter.backtrace_depth_min.map( |value| value as usize );
    output.backtrace_filter.only_backtrace_length_at_most = filter.backtrace_depth_max.map( |value| value as usize );

    if let Some( id ) = filter.backtraces {
        output.backtrace_filter.only_matching_backtraces = Some( std::iter::once( BacktraceId::new( id ) ).collect() );
    }

    if let Some( id ) = filter.deallocation_backtraces {
        output.backtrace_filter.only_matching_deallocation_backtraces = Some( std::iter::once( BacktraceId::new( id ) ).collect() );
    }

    match filter.mmaped {
        None => {},
        Some( protocol::MmapedFilter::Yes ) => output.only_ptmalloc_mmaped = true,
        Some( protocol::MmapedFilter::No ) => output.only_ptmalloc_not_mmaped = true
    }

    match filter.jemalloc {
        None => {},
        Some( protocol::JemallocFilter::Yes ) => output.only_jemalloc = true,
        Some( protocol::JemallocFilter::No ) => output.only_not_jemalloc = true
    }

    match filter.arena {
        None => {},
        Some( protocol::ArenaFilter::Main ) => output.only_ptmalloc_from_main_arena = true,
        Some( protocol::ArenaFilter::NonMain ) => output.only_ptmalloc_not_from_main_arena = true
    }

    if let Some( ref pattern ) = filter.function_regex {
        output.backtrace_filter.only_passing_through_function = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "function_regex", err ) )?
        );
    }

    if let Some( ref pattern ) = filter.negative_function_regex {
        output.backtrace_filter.only_not_passing_through_function = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "negative_function_regex", err ) )?
        );
    }

    if let Some( ref pattern ) = filter.source_regex {
        output.backtrace_filter.only_passing_through_source = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "source_regex", err ) )?
        );
    }

    if let Some( ref pattern ) = filter.negative_source_regex {
        output.backtrace_filter.only_not_passing_through_source = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "negative_source_regex", err ) )?
        );
    }

    output.only_with_marker = filter.marker;

    output.only_group_interval_at_least = filter.group_interval_min.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.only_group_interval_at_most = filter.group_interval_max.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.only_group_max_total_usage_first_seen_at_least = filter.group_max_total_usage_first_seen_min.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.only_group_max_total_usage_first_seen_at_most = filter.group_max_total_usage_first_seen_max.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.only_group_allocations_at_least = filter.group_allocations_min.map( |value| value as usize );
    output.only_group_allocations_at_most = filter.group_allocations_max.map( |value| value as usize );
    output.only_group_leaked_allocations_at_least = filter.group_leaked_allocations_min.map( |value| value.into() );
    output.only_group_leaked_allocations_at_most = filter.group_leaked_allocations_max.map( |value| value.into() );

    output.only_chain_length_at_least = filter.chain_length_min;
    output.only_chain_length_at_most = filter.chain_length_max;
    output.only_chain_alive_for_at_least = filter.chain_lifetime_min.map( |interval| Duration( interval.0 ) );
    output.only_chain_alive_for_at_most = filter.chain_lifetime_max.map( |interval| Duration( interval.0 ) );
    output.only_position_in_chain_at_least = filter.position_in_chain_min;
    output.only_position_in_chain_at_most = filter.position_in_chain_max;

    match filter.lifetime.unwrap_or( protocol::LifetimeFilter::All ) {
        protocol::LifetimeFilter::All => {},
        protocol::LifetimeFilter::OnlyLeaked => {
            output.common_filter.only_leaked = true;
        },
        protocol::LifetimeFilter::OnlyChainLeaked => {
            output.only_chain_leaked = true;
        },
        protocol::LifetimeFilter::OnlyTemporary => {
            output.common_filter.only_temporary = true;
        },
        protocol::LifetimeFilter::OnlyWholeGroupLeaked => {
            output.only_group_leaked_allocations_at_least = Some( cli_core::NumberOrFractionOfTotal::Fraction( 1.0 ) );
        }
    }

    let output: cli_core::AllocationFilter = output.into();
    Ok( output )
}

#[derive(Clone)]
pub struct MapFilter {
    filter: cli_core::CompiledMapFilter,
    custom_filter: Option< Arc< HashSet< MapId > > >
}

impl MapFilter {
    pub fn try_match( &self, data: &Data, id: MapId, allocation: &Map ) -> bool {
        if let Some( ref custom_filter ) = self.custom_filter {
            if !custom_filter.contains( &id ) {
                return false;
            }
        }

        if !self.filter.try_match( data, allocation ) {
            return false;
        }

        true
    }
}

pub fn prepare_map_filter(
    data: &Arc< Data >,
    filter: &protocol::MapFilter,
    custom_filter: &protocol::CustomFilter
) -> Result< MapFilter, PrepareFilterError > {
    let filter = prepare_raw_map_filter( data, filter )?.compile( data );
    let custom_filter = run_custom_map_filter( data, custom_filter ).map_err( |error| PrepareFilterError::InvalidCustomFilter( error.message ) )?;

    Ok( MapFilter { filter, custom_filter } )
}

pub fn prepare_raw_map_filter( data: &Data, filter: &protocol::MapFilter ) -> Result< cli_core::MapFilter, PrepareFilterError > {
    use cli_core::Duration;

    let mut output = cli_core::RawMapFilter::default();

    // TODO: Deduplicate this with the allocation filter.
    output.common_filter.only_allocated_after_at_least = filter.from.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.common_filter.only_allocated_until_at_most = filter.to.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) );
    output.common_filter.only_address_at_least = filter.address_min;
    output.common_filter.only_address_at_most = filter.address_max;
    output.common_filter.only_larger_or_equal = filter.size_min;
    output.common_filter.only_smaller_or_equal = filter.size_max;

    if let Some( only_alive_at ) = filter.alive_at.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) ) {
        output.common_filter.only_alive_at.push( only_alive_at );
    }
    if let Some( only_alive_at ) = filter.alive_at_2.map( |ts| Duration( ts.to_timestamp( data.initial_timestamp(), data.last_timestamp() ) ) ) {
        output.common_filter.only_alive_at.push( only_alive_at );
    }

    output.common_filter.only_alive_for_at_least = filter.lifetime_min.map( |interval| Duration( interval.0 ) );
    output.common_filter.only_alive_for_at_most = filter.lifetime_max.map( |interval| Duration( interval.0 ) );

    output.backtrace_filter.only_backtrace_length_at_least = filter.backtrace_depth_min.map( |value| value as usize );
    output.backtrace_filter.only_backtrace_length_at_most = filter.backtrace_depth_max.map( |value| value as usize );

    if let Some( id ) = filter.backtraces {
        output.backtrace_filter.only_matching_backtraces = Some( std::iter::once( BacktraceId::new( id ) ).collect() );
    }

    if let Some( id ) = filter.deallocation_backtraces {
        output.backtrace_filter.only_matching_deallocation_backtraces = Some( std::iter::once( BacktraceId::new( id ) ).collect() );
    }

    if let Some( ref pattern ) = filter.function_regex {
        output.backtrace_filter.only_passing_through_function = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "function_regex", err ) )?
        );
    }

    if let Some( ref pattern ) = filter.negative_function_regex {
        output.backtrace_filter.only_not_passing_through_function = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "negative_function_regex", err ) )?
        );
    }

    if let Some( ref pattern ) = filter.source_regex {
        output.backtrace_filter.only_passing_through_source = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "source_regex", err ) )?
        );
    }

    if let Some( ref pattern ) = filter.negative_source_regex {
        output.backtrace_filter.only_not_passing_through_source = Some(
            Regex::new( &pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "negative_source_regex", err ) )?
        );
    }

    match filter.lifetime.unwrap_or( protocol::LifetimeFilter::All ) {
        protocol::LifetimeFilter::All => {},
        protocol::LifetimeFilter::OnlyLeaked => {
            output.common_filter.only_leaked = true;
        },
        protocol::LifetimeFilter::OnlyChainLeaked => {
            unimplemented!()
        },
        protocol::LifetimeFilter::OnlyTemporary => {
            output.common_filter.only_temporary = true;
        },
        protocol::LifetimeFilter::OnlyWholeGroupLeaked => {
            unimplemented!()
        }
    }

    output.only_peak_rss_at_least = filter.peak_rss_min;
    output.only_peak_rss_at_most = filter.peak_rss_max;
    output.only_jemalloc = filter.jemalloc == Some( protocol::BoolFilter::OnlyTrue );
    output.only_not_jemalloc = filter.jemalloc == Some( protocol::BoolFilter::OnlyFalse );
    output.only_bytehound = filter.bytehound == Some( protocol::BoolFilter::OnlyTrue );
    output.only_not_bytehound = filter.bytehound == Some( protocol::BoolFilter::OnlyFalse );
    output.only_readable = filter.readable == Some( protocol::BoolFilter::OnlyTrue );
    output.only_not_readable = filter.readable == Some( protocol::BoolFilter::OnlyFalse );
    output.only_writable = filter.writable == Some( protocol::BoolFilter::OnlyTrue );
    output.only_not_writable = filter.writable == Some( protocol::BoolFilter::OnlyFalse );
    output.only_executable = filter.executable == Some( protocol::BoolFilter::OnlyTrue );
    output.only_not_executable = filter.executable == Some( protocol::BoolFilter::OnlyFalse );

    let output: cli_core::MapFilter = output.into();
    Ok( output )
}

pub enum PrepareFilterError {
    InvalidRegex( &'static str, regex::Error ),
    InvalidCustomFilter( String )
}

pub fn prepare_backtrace_filter( filter: &protocol::BacktraceFilter ) -> Result< BacktraceFilter, PrepareFilterError > {
    let function_regex = if let Some( ref pattern ) = filter.function_regex {
        Some( Regex::new( pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "function_regex", err ) )? )
    } else {
        None
    };

    let source_regex = if let Some( ref pattern ) = filter.source_regex {
        Some( Regex::new( pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "source_regex", err ) )? )
    } else {
        None
    };

    let negative_function_regex = if let Some( ref pattern ) = filter.negative_function_regex {
        Some( Regex::new( pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "negative_function_regex", err ) )? )
    } else {
        None
    };

    let negative_source_regex = if let Some( ref pattern ) = filter.negative_source_regex {
        Some( Regex::new( pattern ).map_err( |err| PrepareFilterError::InvalidRegex( "negative_source_regex", err ) )? )
    } else {
        None
    };

    let filter = BacktraceFilter {
        backtrace_depth_min: filter.backtrace_depth_min.unwrap_or( 0 ) as usize,
        backtrace_depth_max: filter.backtrace_depth_max.unwrap_or( std::u32::MAX ) as usize,
        function_regex,
        source_regex,
        negative_function_regex,
        negative_source_regex
    };

    Ok( filter )
}

pub fn match_backtrace< 'a >(
    data: &Data,
    positive_cache: &mut HashMap< crate::FrameId, bool >,
    negative_cache: &mut HashMap< crate::FrameId, bool >,
    filter: &BacktraceFilter,
    backtrace: impl ExactSizeIterator< Item = (crate::FrameId, &'a crate::Frame) >
) -> bool {
    if backtrace.len() < filter.backtrace_depth_min || backtrace.len() > filter.backtrace_depth_max {
        return false;
    }

    let mut positive_matched = filter.function_regex.is_none() && filter.source_regex.is_none();
    let mut negative_matched = false;
    let check_negative = filter.negative_function_regex.is_some() || filter.negative_source_regex.is_some();

    for (frame_id, frame) in backtrace {
        let check_positive =
            if positive_matched {
                false
            } else if let Some( &cached_result ) = positive_cache.get( &frame_id ) {
                positive_matched = cached_result;
                false
            } else {
                true
            };

        if positive_matched && !check_negative {
            break;
        }

        let mut function = None;
        if (check_positive && filter.function_regex.is_some()) || filter.negative_function_regex.is_some() {
            function = frame.function().or_else( || frame.raw_function() ).map( |id| data.interner().resolve( id ).unwrap() );
        }

        let mut source = None;
        if (check_positive && filter.source_regex.is_some()) || filter.negative_source_regex.is_some() {
            source = frame.source().map( |id| data.interner().resolve( id ).unwrap() )
        }

        if check_positive {
            let matched_function =
                if let Some( regex ) = filter.function_regex.as_ref() {
                    if let Some( ref function ) = function {
                        regex.is_match( function )
                    } else {
                        false
                    }
                } else {
                    true
                };

            let matched_source =
                if let Some( regex ) = filter.source_regex.as_ref() {
                    if let Some( ref source ) = source {
                        regex.is_match( source )
                    } else {
                        false
                    }
                } else {
                    true
                };

            positive_matched = matched_function && matched_source;
            positive_cache.insert( frame_id, positive_matched );
        }

        if check_negative {
            match negative_cache.get( &frame_id ).cloned() {
                Some( true ) => {
                    negative_matched = true;
                    break;
                },
                Some( false ) => {
                    continue;
                },
                None => {}
            }

            if let Some( regex ) = filter.negative_function_regex.as_ref() {
                if let Some( ref function ) = function {
                    if regex.is_match( function ) {
                        negative_cache.insert( frame_id, true );
                        negative_matched = true;
                        break;
                    }
                }
            }

            if let Some( regex ) = filter.negative_source_regex.as_ref() {
                if let Some( ref source ) = source {
                    if regex.is_match( source ) {
                        negative_cache.insert( frame_id, true );
                        negative_matched = true;
                        break;
                    }
                }
            }

            negative_cache.insert( frame_id, false );
        }
    }

    positive_matched && !negative_matched
}
