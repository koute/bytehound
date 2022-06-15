use regex::Regex;
use ahash::AHashMap as HashMap;
use ahash::AHashSet as HashSet;
use crate::{Allocation, BacktraceId, Data, Timestamp};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Duration( pub common::Timestamp );

impl Duration {
    pub fn from_secs( value: u64 ) -> Self {
        Self( common::Timestamp::from_secs( value ) )
    }

    pub fn from_usecs( value: u64 ) -> Self {
        Self( common::Timestamp::from_usecs( value ) )
    }

    pub fn from_msecs( value: u64 ) -> Self {
        Self( common::Timestamp::from_msecs( value ) )
    }
}

#[derive(Clone, Default)]
pub struct BasicFilter {
    pub only_passing_through_function: Option< Regex >,
    pub only_not_passing_through_function: Option< Regex >,
    pub only_passing_through_source: Option< Regex >,
    pub only_not_passing_through_source: Option< Regex >,
    pub only_matching_backtraces: Option< HashSet< BacktraceId > >,
    pub only_not_matching_backtraces: Option< HashSet< BacktraceId > >,
    pub only_backtrace_length_at_least: Option< usize >,
    pub only_backtrace_length_at_most: Option< usize >,

    pub only_matching_deallocation_backtraces: Option< HashSet< BacktraceId > >,
    pub only_not_matching_deallocation_backtraces: Option< HashSet< BacktraceId > >,

    pub only_larger_or_equal: Option< u64 >,
    pub only_larger: Option< u64 >,
    pub only_smaller_or_equal: Option< u64 >,
    pub only_smaller: Option< u64 >,

    pub only_address_at_least: Option< u64 >,
    pub only_address_at_most: Option< u64 >,
    pub only_allocated_after_at_least: Option< Duration >,
    pub only_allocated_until_at_most: Option< Duration >,
    pub only_deallocated_after_at_least: Option< Duration >,
    pub only_deallocated_until_at_most: Option< Duration >,
    pub only_not_deallocated_after_at_least: Option< Duration >,
    pub only_not_deallocated_until_at_most: Option< Duration >,
    pub only_alive_for_at_least: Option< Duration >,
    pub only_alive_for_at_most: Option< Duration >,
    pub only_leaked_or_deallocated_after: Option< Duration >,

    pub only_first_size_larger_or_equal: Option< u64 >,
    pub only_first_size_larger: Option< u64 >,
    pub only_first_size_smaller_or_equal: Option< u64 >,
    pub only_first_size_smaller: Option< u64 >,
    pub only_last_size_larger_or_equal: Option< u64 >,
    pub only_last_size_larger: Option< u64 >,
    pub only_last_size_smaller_or_equal: Option< u64 >,
    pub only_last_size_smaller: Option< u64 >,
    pub only_chain_length_at_least: Option< u32 >,
    pub only_chain_length_at_most: Option< u32 >,
    pub only_chain_alive_for_at_least: Option< Duration >,
    pub only_chain_alive_for_at_most: Option< Duration >,

    pub only_group_allocations_at_least: Option< usize >,
    pub only_group_allocations_at_most: Option< usize >,
    pub only_group_interval_at_least: Option< Duration >,
    pub only_group_interval_at_most: Option< Duration >,
    pub only_group_max_total_usage_first_seen_at_least: Option< Duration >,
    pub only_group_max_total_usage_first_seen_at_most: Option< Duration >,
    pub only_group_leaked_allocations_at_least: Option< NumberOrFractionOfTotal >,
    pub only_group_leaked_allocations_at_most: Option< NumberOrFractionOfTotal >,

    pub only_leaked: bool,
    pub only_chain_leaked: bool,
    pub only_temporary: bool,
    pub only_chain_temporary: bool,
    pub only_ptmalloc_mmaped: bool,
    pub only_ptmalloc_not_mmaped: bool,
    pub only_ptmalloc_from_main_arena: bool,
    pub only_ptmalloc_not_from_main_arena: bool,
    pub only_jemalloc: bool,
    pub only_not_jemalloc: bool,
    pub only_with_marker: Option< u32 >
}

#[derive(Copy, Clone)]
pub enum NumberOrFractionOfTotal {
    Number( u64 ),
    Fraction( f64 )
}

impl NumberOrFractionOfTotal {
    pub fn get( self, total: u64 ) -> u64 {
        match self {
            NumberOrFractionOfTotal::Number( value ) => value,
            NumberOrFractionOfTotal::Fraction( fraction ) => (total as f64 * fraction) as u64
        }
    }
}

#[derive(Clone)]
pub struct CompiledBasicFilter {
    is_impossible: bool,

    only_backtraces: Option< HashSet< BacktraceId > >,
    only_not_matching_backtraces: Option< HashSet< BacktraceId > >,

    only_deallocation_backtraces: Option< HashSet< BacktraceId > >,
    only_not_matching_deallocation_backtraces: Option< HashSet< BacktraceId > >,

    only_larger_or_equal: u64,
    only_smaller_or_equal: u64,
    only_address_at_least: u64,
    only_address_at_most: u64,
    only_allocated_after_at_least: Timestamp,
    only_allocated_until_at_most: Timestamp,
    only_deallocated_between_inclusive: Option< (Timestamp, Timestamp) >,
    only_not_deallocated_after_at_least: Option< Timestamp >,
    only_not_deallocated_until_at_most: Option< Timestamp >,
    only_alive_for_at_least: Duration,
    only_alive_for_at_most: Option< Duration >,
    only_leaked_or_deallocated_after: Timestamp,

    enable_chain_filter: bool,
    only_first_size_larger_or_equal: u64,
    only_first_size_smaller_or_equal: u64,
    only_last_size_larger_or_equal: u64,
    only_last_size_smaller_or_equal: u64,
    only_chain_length_at_least: u32,
    only_chain_length_at_most: u32,
    only_chain_alive_for_at_least: Duration,
    only_chain_alive_for_at_most: Option< Duration >,
    only_chain_leaked_or_deallocated_after: Timestamp,
    only_chain_deallocated_between_inclusive: Option< (Timestamp, Timestamp) >,

    enable_group_filter: bool,
    only_group_allocations_at_least: usize,
    only_group_allocations_at_most: usize,
    only_group_interval_at_least: Duration,
    only_group_interval_at_most: Duration,
    only_group_max_total_usage_first_seen_at_least: Timestamp,
    only_group_max_total_usage_first_seen_at_most: Timestamp,
    only_group_leaked_allocations_at_least: NumberOrFractionOfTotal,
    only_group_leaked_allocations_at_most: NumberOrFractionOfTotal,

    only_ptmalloc_mmaped: Option< bool >,
    only_ptmalloc_from_main_arena: Option< bool >,
    only_jemalloc: Option< bool >,
    only_with_marker: Option< u32 >
}

impl From< BasicFilter > for Filter {
    fn from( filter: BasicFilter ) -> Self {
        Filter::Basic( filter )
    }
}

#[derive(Clone)]
pub enum Filter {
    Basic( BasicFilter ),
    And( Box< Filter >, Box< Filter > ),
    Or( Box< Filter >, Box< Filter > ),
    Not( Box< Filter > ),
}

#[derive(Clone)]
pub enum CompiledFilter {
    Basic( CompiledBasicFilter ),
    And( Box< CompiledFilter >, Box< CompiledFilter > ),
    Or( Box< CompiledFilter >, Box< CompiledFilter > ),
    Not( Box< CompiledFilter > ),
}

fn compile_backtrace_filter( data: &Data, filter: &BasicFilter ) -> Option< HashSet< BacktraceId > > {
    let is_none =
        filter.only_passing_through_function.is_none() &&
        filter.only_not_passing_through_function.is_none() &&
        filter.only_passing_through_source.is_none() &&
        filter.only_not_passing_through_source.is_none() &&
        filter.only_backtrace_length_at_least.is_none() &&
        filter.only_backtrace_length_at_most.is_none();

    if is_none {
        return filter.only_matching_backtraces.clone();
    }

    let only_backtrace_length_at_least = filter.only_backtrace_length_at_least.unwrap_or( 0 );
    let only_backtrace_length_at_most = filter.only_backtrace_length_at_most.unwrap_or( !0 );

    let mut matched_backtraces = HashSet::new();
    let mut positive_cache = HashMap::new();
    let mut negative_cache = HashMap::new();
    for (backtrace_id, backtrace) in data.all_backtraces() {
        if backtrace.len() < only_backtrace_length_at_least || backtrace.len() > only_backtrace_length_at_most {
            continue;
        }

        let mut positive_matched =
            filter.only_passing_through_function.is_none() &&
            filter.only_passing_through_source.is_none();
        let mut negative_matched = false;
        let check_negative =
            filter.only_not_passing_through_function.is_some() ||
            filter.only_not_passing_through_source.is_some();

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
            if (check_positive && filter.only_passing_through_function.is_some()) || filter.only_not_passing_through_function.is_some() {
                function = frame.function().or_else( || frame.raw_function() ).map( |id| data.interner().resolve( id ).unwrap() );
            }

            let mut source = None;
            if (check_positive && filter.only_passing_through_source.is_some()) || filter.only_not_passing_through_source.is_some() {
                source = frame.source().map( |id| data.interner().resolve( id ).unwrap() )
            }

            if check_positive {
                let matched_function =
                    if let Some( regex ) = filter.only_passing_through_function.as_ref() {
                        if let Some( ref function ) = function {
                            regex.is_match( function )
                        } else {
                            false
                        }
                    } else {
                        true
                    };

                let matched_source =
                    if let Some( regex ) = filter.only_passing_through_source.as_ref() {
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

                if let Some( regex ) = filter.only_not_passing_through_function.as_ref() {
                    if let Some( ref function ) = function {
                        if regex.is_match( function ) {
                            negative_cache.insert( frame_id, true );
                            negative_matched = true;
                            break;
                        }
                    }
                }

                if let Some( regex ) = filter.only_not_passing_through_source.as_ref() {
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

        if positive_matched && !negative_matched {
            matched_backtraces.insert( backtrace_id );
        }
    }

    if let Some( ref only_matching_backtraces ) = filter.only_matching_backtraces {
        matched_backtraces = matched_backtraces.intersection( &only_matching_backtraces ).copied().collect();
    }

    Some( matched_backtraces )
}

impl BasicFilter {
    fn compile( &self, data: &Data ) -> CompiledBasicFilter {
        let mut is_impossible = false;
        let only_backtraces = compile_backtrace_filter( data, self );

        let mut only_larger_or_equal = self.only_larger_or_equal.unwrap_or( 0 );
        if let Some( only_larger ) = self.only_larger {
            if only_larger == !0 {
                is_impossible = true;
            } else {
                only_larger_or_equal = std::cmp::max( only_larger_or_equal, only_larger + 1 );
            }
        }

        let mut only_smaller_or_equal = self.only_smaller_or_equal.unwrap_or( !0 );
        if let Some( only_smaller ) = self.only_smaller {
            if only_smaller == 0 {
                is_impossible = true;
            } else {
                only_smaller_or_equal = std::cmp::min( only_smaller_or_equal, only_smaller - 1 );
            }
        }

        let mut only_deallocated_between_inclusive = None;
        if self.only_deallocated_after_at_least.is_some() || self.only_deallocated_until_at_most.is_some() {
            only_deallocated_between_inclusive = Some((
                self.only_deallocated_after_at_least.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.initial_timestamp ),
                self.only_deallocated_until_at_most.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.last_timestamp )
            ));
        }

        let mut only_first_size_larger_or_equal = self.only_first_size_larger_or_equal.unwrap_or( 0 );
        if let Some( only_first_size_larger ) = self.only_first_size_larger {
            if only_first_size_larger == !0 {
                is_impossible = true;
            } else {
                only_first_size_larger_or_equal = std::cmp::max( only_first_size_larger_or_equal, only_first_size_larger + 1 );
            }
        }

        let mut only_first_size_smaller_or_equal = self.only_first_size_smaller_or_equal.unwrap_or( !0 );
        if let Some( only_first_size_smaller ) = self.only_first_size_smaller {
            if only_first_size_smaller == 0 {
                is_impossible = true;
            } else {
                only_first_size_smaller_or_equal = std::cmp::min( only_first_size_smaller_or_equal, only_first_size_smaller - 1 );
            }
        }

        let mut only_last_size_larger_or_equal = self.only_last_size_larger_or_equal.unwrap_or( 0 );
        if let Some( only_last_size_larger ) = self.only_last_size_larger {
            if only_last_size_larger == !0 {
                is_impossible = true;
            } else {
                only_last_size_larger_or_equal = std::cmp::max( only_last_size_larger_or_equal, only_last_size_larger + 1 );
            }
        }

        let mut only_last_size_smaller_or_equal = self.only_last_size_smaller_or_equal.unwrap_or( !0 );
        if let Some( only_last_size_smaller ) = self.only_last_size_smaller {
            if only_last_size_smaller == 0 {
                is_impossible = true;
            } else {
                only_last_size_smaller_or_equal = std::cmp::min( only_last_size_smaller_or_equal, only_last_size_smaller - 1 );
            }
        }

        let mut only_leaked_or_deallocated_after = self.only_leaked_or_deallocated_after.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.initial_timestamp );

        if self.only_leaked && self.only_temporary {
            is_impossible = true;
        }

        if self.only_ptmalloc_mmaped && self.only_ptmalloc_not_mmaped {
            is_impossible = true;
        }

        if self.only_ptmalloc_from_main_arena && self.only_ptmalloc_not_from_main_arena {
            is_impossible = true;
        }

        if self.only_jemalloc && self.only_not_jemalloc {
            is_impossible = true;
        }

        if self.only_leaked {
            only_leaked_or_deallocated_after = data.last_timestamp;
        }

        if self.only_temporary {
            if let Some( (ref mut min, ref mut max) ) = only_deallocated_between_inclusive {
                *min = std::cmp::max( *min, data.initial_timestamp );
                *max = std::cmp::min( *max, data.last_timestamp );
            } else {
                only_deallocated_between_inclusive = Some( (data.initial_timestamp, data.last_timestamp) );
            }
        }

        let enable_chain_filter =
            self.only_first_size_larger.is_some() ||
            self.only_first_size_larger_or_equal.is_some() ||
            self.only_first_size_smaller.is_some() ||
            self.only_first_size_smaller_or_equal.is_some() ||
            self.only_last_size_larger.is_some() ||
            self.only_last_size_larger_or_equal.is_some() ||
            self.only_last_size_smaller.is_some() ||
            self.only_last_size_smaller_or_equal.is_some() ||
            self.only_chain_length_at_least.is_some() ||
            self.only_chain_length_at_most.is_some() ||
            self.only_chain_alive_for_at_least.is_some() ||
            self.only_chain_alive_for_at_most.is_some() ||
            self.only_chain_leaked ||
            self.only_chain_temporary;

        let mut only_chain_leaked_or_deallocated_after = data.initial_timestamp;
        if self.only_chain_leaked {
            only_chain_leaked_or_deallocated_after = data.last_timestamp;
        }

        let mut only_chain_deallocated_between_inclusive = None;
        if self.only_chain_temporary {
            if let Some( (ref mut min, ref mut max) ) = only_chain_deallocated_between_inclusive {
                *min = std::cmp::max( *min, data.initial_timestamp );
                *max = std::cmp::min( *max, data.last_timestamp );
            } else {
                only_chain_deallocated_between_inclusive = Some( (data.initial_timestamp, data.last_timestamp) );
            }
        }

        let enable_group_filter =
            self.only_group_allocations_at_least.is_some() ||
            self.only_group_allocations_at_most.is_some() ||
            self.only_group_interval_at_least.is_some() ||
            self.only_group_interval_at_most.is_some() ||
            self.only_group_max_total_usage_first_seen_at_least.is_some() ||
            self.only_group_max_total_usage_first_seen_at_most.is_some() ||
            self.only_group_leaked_allocations_at_least.is_some() ||
            self.only_group_leaked_allocations_at_most.is_some();

        CompiledBasicFilter {
            is_impossible,

            only_backtraces,
            only_not_matching_backtraces: self.only_not_matching_backtraces.clone(),

            only_deallocation_backtraces: self.only_matching_deallocation_backtraces.clone(),
            only_not_matching_deallocation_backtraces: self.only_not_matching_deallocation_backtraces.clone(),

            only_larger_or_equal,
            only_smaller_or_equal,
            only_address_at_least: self.only_address_at_least.unwrap_or( 0 ),
            only_address_at_most: self.only_address_at_most.unwrap_or( !0 ),
            only_allocated_after_at_least: self.only_allocated_after_at_least.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.initial_timestamp ),
            only_allocated_until_at_most: self.only_allocated_until_at_most.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.last_timestamp ),
            only_deallocated_between_inclusive: only_deallocated_between_inclusive,
            only_not_deallocated_after_at_least: self.only_not_deallocated_after_at_least.map( |offset| data.initial_timestamp + offset.0 ),
            only_not_deallocated_until_at_most: self.only_not_deallocated_until_at_most.map( |offset| data.initial_timestamp + offset.0 ),
            only_alive_for_at_least: self.only_alive_for_at_least.unwrap_or( Duration::from_secs( 0 ) ),
            only_alive_for_at_most: self.only_alive_for_at_most,
            only_leaked_or_deallocated_after,

            enable_chain_filter,
            only_first_size_larger_or_equal,
            only_first_size_smaller_or_equal,
            only_last_size_larger_or_equal,
            only_last_size_smaller_or_equal,
            only_chain_length_at_least: self.only_chain_length_at_least.unwrap_or( 0 ),
            only_chain_length_at_most: self.only_chain_length_at_most.unwrap_or( !0 ),
            only_chain_alive_for_at_least: self.only_chain_alive_for_at_least.unwrap_or( Duration::from_secs( 0 ) ),
            only_chain_alive_for_at_most: self.only_chain_alive_for_at_most,
            only_chain_leaked_or_deallocated_after,
            only_chain_deallocated_between_inclusive,

            only_group_allocations_at_least: self.only_group_allocations_at_least.unwrap_or( 0 ),
            only_group_allocations_at_most: self.only_group_allocations_at_most.unwrap_or( !0 ),
            only_group_interval_at_least: self.only_group_interval_at_least.unwrap_or( Duration::from_secs( 0 ) ),
            only_group_interval_at_most: self.only_group_interval_at_most.unwrap_or( Duration::from_secs( 5000 * 365 * 24 * 3600 ) ),
            only_group_max_total_usage_first_seen_at_least: self.only_group_max_total_usage_first_seen_at_least.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.initial_timestamp ),
            only_group_max_total_usage_first_seen_at_most: self.only_group_max_total_usage_first_seen_at_most.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.last_timestamp ),
            only_group_leaked_allocations_at_least: self.only_group_leaked_allocations_at_least.unwrap_or( NumberOrFractionOfTotal::Number( 0 ) ),
            only_group_leaked_allocations_at_most: self.only_group_leaked_allocations_at_most.unwrap_or( NumberOrFractionOfTotal::Number( !0 ) ),

            enable_group_filter,

            only_ptmalloc_mmaped:
                if self.only_ptmalloc_mmaped {
                    Some( true )
                } else if self.only_ptmalloc_not_mmaped {
                    Some( false )
                } else {
                    None
                },
            only_ptmalloc_from_main_arena:
                if self.only_ptmalloc_from_main_arena {
                    Some( true )
                } else if self.only_ptmalloc_not_from_main_arena {
                    Some( false )
                } else {
                    None
                },
            only_jemalloc:
                if self.only_jemalloc {
                    Some( true )
                } else if self.only_not_jemalloc {
                    Some( false )
                } else {
                    None
                },
            only_with_marker: self.only_with_marker
        }
    }
}

impl CompiledBasicFilter {
    fn try_match( &self, data: &Data, allocation: &Allocation ) -> bool {
        if self.is_impossible {
            return false;
        }

        if !(allocation.size <= self.only_smaller_or_equal && allocation.size >= self.only_larger_or_equal) {
            return false;
        }

        if !(allocation.pointer >= self.only_address_at_least && allocation.pointer <= self.only_address_at_most) {
            return false;
        }

        if !(allocation.timestamp >= self.only_allocated_after_at_least && allocation.timestamp <= self.only_allocated_until_at_most) {
            return false;
        }

        if let Some( (min, max) ) = self.only_deallocated_between_inclusive {
            if let Some( ref deallocation ) = allocation.deallocation {
                if !(deallocation.timestamp >= min && deallocation.timestamp <= max) {
                    return false;
                }
            } else {
                return false;
            }
        }

        let lifetime_end = allocation.deallocation.as_ref().map( |deallocation| deallocation.timestamp ).unwrap_or( data.last_timestamp() );
        let lifetime = Duration( lifetime_end - allocation.timestamp );

        if lifetime < self.only_alive_for_at_least {
            return false;
        }

        if let Some( max ) = self.only_alive_for_at_most {
            if lifetime > max {
                return false;
            }
        }

        if let Some( ref deallocation ) = allocation.deallocation {
            if !(deallocation.timestamp > self.only_leaked_or_deallocated_after) {
                return false;
            }

            if let Some( only_not_deallocated_after_at_least ) = self.only_not_deallocated_after_at_least {
                if deallocation.timestamp >= only_not_deallocated_after_at_least {
                    return false;
                }
            }

            if let Some( only_not_deallocated_until_at_most ) = self.only_not_deallocated_until_at_most {
                if deallocation.timestamp <= only_not_deallocated_until_at_most {
                    return false;
                }
            }
        }

        if let Some( ref only_backtraces ) = self.only_backtraces {
            if !only_backtraces.contains( &allocation.backtrace ) {
                return false;
            }
        }

        if let Some( ref set ) = self.only_not_matching_backtraces {
            if set.contains( &allocation.backtrace ) {
                return false;
            }
        }

        if let Some( ref only_deallocation_backtraces ) = self.only_deallocation_backtraces {
            if let Some( ref deallocation ) = allocation.deallocation {
                if let Some( backtrace ) = deallocation.backtrace {
                    if !only_deallocation_backtraces.contains( &backtrace ) {
                        return false;
                    }
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }

        if let Some( ref set ) = self.only_not_matching_deallocation_backtraces {
            if let Some( ref deallocation ) = allocation.deallocation {
                if let Some( backtrace ) = deallocation.backtrace {
                    if set.contains( &backtrace ) {
                        return false;
                    }
                }
            }
        }

        if self.enable_chain_filter {
            let first_allocation_size;
            let last_allocation_size;
            let chain_length;
            let chain_lifetime;
            let chain_lifetime_end;
            let was_deallocated;
            if let Some( first_in_chain ) = allocation.first_allocation_in_chain {
                let chain = data.get_chain_by_first_allocation( first_in_chain ).unwrap();
                let first_allocation = data.get_allocation( chain.first );
                let last_allocation = data.get_allocation( chain.last );

                first_allocation_size = first_allocation.size;
                last_allocation_size = last_allocation.size;
                chain_length = chain.length;

                chain_lifetime_end = last_allocation.deallocation.as_ref().map( |deallocation| deallocation.timestamp ).unwrap_or( data.last_timestamp() );
                chain_lifetime = Duration( chain_lifetime_end - first_allocation.timestamp );
                was_deallocated = last_allocation.deallocation.is_some();
            } else {
                first_allocation_size = allocation.size;
                last_allocation_size = allocation.size;
                chain_length = 1;
                chain_lifetime = lifetime;
                chain_lifetime_end = allocation.deallocation.as_ref().map( |deallocation| deallocation.timestamp ).unwrap_or( data.last_timestamp() );
                was_deallocated = allocation.deallocation.is_some();
            }

            if !(first_allocation_size <= self.only_first_size_smaller_or_equal && first_allocation_size >= self.only_first_size_larger_or_equal) {
                return false;
            }

            if !(last_allocation_size <= self.only_last_size_smaller_or_equal && last_allocation_size >= self.only_last_size_larger_or_equal) {
                return false;
            }

            if !(chain_length >= self.only_chain_length_at_least && chain_length <= self.only_chain_length_at_most) {
                return false;
            }

            if chain_lifetime < self.only_chain_alive_for_at_least {
                return false;
            }

            if let Some( max ) = self.only_chain_alive_for_at_most {
                if chain_lifetime > max {
                    return false;
                }
            }

            if was_deallocated {
                if !(chain_lifetime_end > self.only_chain_leaked_or_deallocated_after) {
                    return false;
                }
            }

            if let Some( (min, max) ) = self.only_chain_deallocated_between_inclusive {
                if !was_deallocated {
                    return false;
                }

                if !(chain_lifetime_end >= min && chain_lifetime_end <= max) {
                    return false;
                }
            }
        }

        if self.enable_group_filter {
            let group_allocations = data.get_allocation_ids_by_backtrace( allocation.backtrace );
            if group_allocations.len() < self.only_group_allocations_at_least {
                return false;
            }

            if group_allocations.len() > self.only_group_allocations_at_most {
                return false;
            }

            let first_timestamp = data.get_allocation( *group_allocations.first().unwrap() ).timestamp;
            let last_timestamp = data.get_allocation( *group_allocations.last().unwrap() ).timestamp;
            let interval = Duration( last_timestamp - first_timestamp );

            if interval < self.only_group_interval_at_least {
                return false;
            }

            if interval > self.only_group_interval_at_most {
                return false;
            }

            let stats = data.get_group_statistics( allocation.backtrace );
            let total_allocations = stats.alloc_count as u64;
            let leaked = (stats.alloc_count - stats.free_count) as u64;

            if leaked < self.only_group_leaked_allocations_at_least.get( total_allocations ) {
                return false;
            }

            if leaked > self.only_group_leaked_allocations_at_most.get( total_allocations ) {
                return false;
            }

            if stats.max_total_usage_first_seen_at < self.only_group_max_total_usage_first_seen_at_least {
                return false;
            }

            if stats.max_total_usage_first_seen_at > self.only_group_max_total_usage_first_seen_at_most {
                return false;
            }
        }

        if let Some( value ) = self.only_ptmalloc_mmaped {
            if allocation.is_jemalloc() {
                return false;
            }

            if allocation.is_mmaped() != value {
                return false;
            }
        }

        if let Some( value ) = self.only_ptmalloc_from_main_arena {
            if allocation.is_jemalloc() {
                return false;
            }

            if !allocation.in_non_main_arena() != value {
                return false;
            }
        }

        if let Some( value ) = self.only_jemalloc {
            if allocation.is_jemalloc() != value {
                return false;
            }
        }

        if let Some( marker ) = self.only_with_marker {
            if allocation.marker != marker {
                return false;
            }
        }

        true
    }
}

impl CompiledFilter {
    pub fn try_match( &self, data: &Data, allocation: &Allocation ) -> bool {
        match *self {
            CompiledFilter::Basic( ref filter ) => filter.try_match( data, allocation ),
            CompiledFilter::And( ref lhs, ref rhs ) => lhs.try_match( data, allocation ) && rhs.try_match( data, allocation ),
            CompiledFilter::Or( ref lhs, ref rhs ) => lhs.try_match( data, allocation ) || rhs.try_match( data, allocation ),
            CompiledFilter::Not( ref filter ) => !filter.try_match( data, allocation )
        }
    }
}

impl Filter {
    pub fn compile( &self, data: &Data ) -> CompiledFilter {
        match *self {
            Filter::Basic( ref filter ) => CompiledFilter::Basic( filter.compile( data ) ),
            Filter::And( ref lhs, ref rhs ) => CompiledFilter::And( Box::new( lhs.compile( data ) ), Box::new( rhs.compile( data ) ) ),
            Filter::Or( ref lhs, ref rhs ) => CompiledFilter::Or( Box::new( lhs.compile( data ) ), Box::new( rhs.compile( data ) ) ),
            Filter::Not( ref filter ) => CompiledFilter::Not( Box::new( filter.compile( data ) ) )
        }
    }
}
