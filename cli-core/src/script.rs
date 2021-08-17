use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::fs::File;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::fmt::Write;
use ahash::AHashMap as HashMap;
use ahash::AHashSet as HashSet;
use rayon::prelude::*;
use parking_lot::Mutex;
use regex::Regex;
use crate::{AllocationId, BacktraceId, Data, Loader};
use crate::data::OperationId;
use crate::exporter_flamegraph_pl::dump_collation_from_iter;
use crate::filter::{BasicFilter, Duration, Filter, NumberOrFractionOfTotal};
use crate::timeline::build_timeline;

pub use rhai;
pub use crate::script_virtual::VirtualEnvironment;
pub use crate::script_virtual::ScriptOutputKind;

struct DecomposedDuration {
    days: u64,
    hours: u64,
    minutes: u64,
    secs: u64,
    ms: u64,
    us: u64
}

impl std::fmt::Display for DecomposedDuration {
    fn fmt( &self, fmt: &mut std::fmt::Formatter ) -> std::fmt::Result {
        let mut non_empty = false;
        if self.days > 0 {
            non_empty = true;
            write!( fmt, "{}d", self.days ).unwrap();
        }
        if self.hours > 0 {
            if non_empty {
                fmt.write_str( " " ).unwrap();
            }
            non_empty = true;
            write!( fmt, "{}h", self.hours ).unwrap();
        }
        if self.minutes > 0 {
            if non_empty {
                fmt.write_str( " " ).unwrap();
            }
            non_empty = true;
            write!( fmt, "{}m", self.minutes ).unwrap();
        }
        if self.secs > 0 {
            if non_empty {
                fmt.write_str( " " ).unwrap();
            }
            non_empty = true;
            write!( fmt, "{}s", self.secs ).unwrap();
        }
        if self.ms > 0 {
            if non_empty {
                fmt.write_str( " " ).unwrap();
            }
            non_empty = true;
            write!( fmt, "{}ms", self.ms ).unwrap();
        }
        if self.us > 0 {
            if non_empty {
                fmt.write_str( " " ).unwrap();
            }
            write!( fmt, "{}us", self.us ).unwrap();
        }

        Ok(())
    }
}

impl Duration {
    fn decompose( self ) -> DecomposedDuration {
        const MS: u64 = 1000;
        const SECOND: u64 = 1000 * MS;
        const MINUTE: u64 = 60 * SECOND;
        const HOUR: u64 = 60 * MINUTE;
        const DAY: u64 = 24 * HOUR;

        let mut us = self.0.as_usecs();
        let days = us / DAY;
        us -= days * DAY;
        let hours = us / HOUR;
        us -= hours * HOUR;
        let minutes = us / MINUTE;
        us -= minutes * MINUTE;
        let secs = us / SECOND;
        us -= secs * SECOND;
        let ms = us / MS;
        us -= ms * MS;

        DecomposedDuration {
            days,
            hours,
            minutes,
            secs,
            ms,
            us
        }
    }
}

fn dirname( path: &str ) -> String {
    match std::path::Path::new( path ).parent() {
        Some( parent ) => {
            parent.to_str().unwrap().into()
        },
        None => {
            ".".into()
        }
    }
}

#[derive(Clone)]
struct DataRef( Arc< Data > );

impl std::fmt::Debug for DataRef {
    fn fmt( &self, fmt: &mut std::fmt::Formatter ) -> std::fmt::Result {
        write!( fmt, "Data" )
    }
}

impl std::ops::Deref for DataRef {
    type Target = Data;
    fn deref( &self ) -> &Self::Target {
        &self.0
    }
}

impl DataRef {
    fn allocations( &mut self ) -> AllocationList {
        AllocationList {
            data: self.clone(),
            allocation_ids: None,
            filter: None
        }
    }
}

#[derive(Clone)]
pub struct AllocationList {
    data: DataRef,
    allocation_ids: Option< Arc< Vec< AllocationId > > >,
    filter: Option< Filter >
}

impl std::fmt::Debug for AllocationList {
    fn fmt( &self, fmt: &mut std::fmt::Formatter ) -> std::fmt::Result {
        write!( fmt, "AllocationList" )
    }
}

// This was copied from the `plotters` crate.
fn gen_keypoints( range: (u64, u64), max_points: usize ) -> Vec< u64 > {
    let mut scale: u64 = 1;
    let range = (range.0.min(range.1), range.0.max(range.1));
    'outer: while (range.1 - range.0 + scale - 1) as usize / (scale as usize) > max_points {
        let next_scale = scale * 10;
        for new_scale in [scale * 2, scale * 5, scale * 10].iter() {
            scale = *new_scale;
            if (range.1 - range.0 + *new_scale - 1) as usize / (*new_scale as usize)
                < max_points
            {
                break 'outer;
            }
        }
        scale = next_scale;
    }

    let (mut left, right) = (
        range.0 + (scale - range.0 % scale) % scale,
        range.1 - range.1 % scale,
    );

    let mut ret = vec![];
    while left <= right {
        ret.push(left as u64);
        left += scale;
    }

    return ret;
}

fn to_chrono( timestamp: u64 ) -> chrono::DateTime< chrono::Utc > {
    use chrono::prelude::*;

    let secs = timestamp / 1_000_000;
    Utc.timestamp( secs as i64, ((timestamp - secs * 1_000_000) * 1000) as u32 )
}

fn expand_datapoints< V >( xs: &[u64], datapoints: &[(u64, V)] ) -> Vec< (u64, V) > where V: Copy + Default {
    if xs.is_empty() {
        return Vec::new();
    }

    if datapoints.is_empty() {
        return xs.iter().map( |&x| (x, Default::default()) ).collect();
    }

    assert!( xs.len() >= datapoints.len() );
    assert!( xs[0] <= datapoints[0].0 );
    assert!( xs[xs.len() - 1] >= datapoints[datapoints.len() - 1].0 );

    let mut expanded = Vec::with_capacity( xs.len() );
    let mut last_value = Default::default();
    let mut dense = xs.iter().copied();
    let mut sparse = datapoints.iter().copied();

    while let Some( mut dense_key ) = dense.next() {
        if let Some( (sparse_key, value) ) = sparse.next() {
            if dense_key < sparse_key {
                while dense_key < sparse_key {
                    expanded.push( (dense_key, last_value) );
                    dense_key = dense.next().unwrap();
                }
            } else if dense_key > sparse_key {
                unreachable!();
            }

            expanded.push( (dense_key, value) );
            last_value = value;
        } else {
            expanded.push( (dense_key, last_value) );
        }
    }

    assert_eq!( xs.len(), expanded.len() );
    expanded
}

#[test]
fn test_expand_datapoints() {
    assert_eq!(
        expand_datapoints( &[0, 1, 2], &[(1, 100)] ),
        &[(0, 0), (1, 100), (2, 100)]
    );

    assert_eq!(
        expand_datapoints( &[0, 1, 2, 3], &[(0, 100), (2, 200)] ),
        &[(0, 100), (1, 100), (2, 200), (3, 200)]
    );
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum OpFilter {
    Both,
    OnlyAlloc,
    None
}

fn get_timestamp( data: &Data, op: OperationId ) -> common::Timestamp {
    if op.is_allocation() || op.is_reallocation() {
        data.get_allocation( op.id() ).timestamp
    } else {
        data.get_allocation( op.id() ).deallocation.as_ref().unwrap().timestamp
    }
}

impl AllocationList {
    pub fn allocation_ids( &mut self ) -> &[AllocationId] {
        self.apply_filter();
        self.unfiltered_allocation_ids()
    }

    fn add_filter_once( &self, is_filled: impl FnOnce( &BasicFilter ) -> bool, callback: impl FnOnce( &mut BasicFilter ) ) -> Self {
        let filter = match self.filter.as_ref() {
            None => {
                let mut new_filter = BasicFilter::default();
                callback( &mut new_filter );

                Filter::Basic( new_filter )
            },
            Some( Filter::Basic( ref old_filter ) ) => {
                if is_filled( old_filter ) {
                    let mut new_filter = BasicFilter::default();
                    callback( &mut new_filter );

                    Filter::And( Box::new( Filter::Basic( old_filter.clone() ) ), Box::new( Filter::Basic( new_filter ) ) )
                } else {
                    let mut new_filter = old_filter.clone();
                    callback( &mut new_filter );

                    Filter::Basic( new_filter )
                }
            },
            Some( Filter::And( ref lhs, ref rhs ) ) if matches!( **rhs, Filter::Basic( _ ) ) => {
                match **rhs {
                    Filter::Basic( ref old_filter ) => {
                        let mut new_filter = old_filter.clone();
                        callback( &mut new_filter );

                        Filter::And( lhs.clone(), Box::new( Filter::Basic( new_filter ) ) )
                    },
                    _ => unreachable!()
                }
            },
            Some( old_filter ) => {
                let mut new_filter = BasicFilter::default();
                callback( &mut new_filter );

                Filter::And( Box::new( old_filter.clone() ), Box::new( Filter::Basic( new_filter ) ) )
            }
        };

        AllocationList {
            data: self.data.clone(),
            allocation_ids: self.allocation_ids.clone(),
            filter: Some( filter )
        }
    }

    fn add_filter( &self, callback: impl FnOnce( &mut BasicFilter ) ) -> Self {
        self.add_filter_once( |_| false, callback )
    }

    fn unfiltered_allocation_ids( &self ) -> &[AllocationId] {
        self.allocation_ids.as_ref().map( |allocation_ids| allocation_ids.as_slice() ).unwrap_or( &self.data.sorted_by_timestamp )
    }

    fn filtered_allocation_ids< 'a >( &'a self ) -> impl ParallelIterator< Item = AllocationId > + 'a {
        let filter = self.filter.as_ref().map( |filter| filter.compile( &self.data ) );
        self.unfiltered_allocation_ids().par_iter().filter( move |id| {
            let allocation = self.data.get_allocation( **id );
            if let Some( ref filter ) = filter {
                filter.try_match( &self.data, allocation )
            } else {
                true
            }
        }).copied()
    }

    fn apply_filter( &mut self ) {
        if self.filter.is_none() {
            return;
        }

        let list: Vec< _ > = self.filtered_allocation_ids().collect();
        self.allocation_ids = Some( Arc::new( list ) );
    }

    fn save_as_flamegraph_to_string( &mut self ) -> Result< String, Box< rhai::EvalAltResult > > {
        self.apply_filter();

        let mut lines = Vec::new();
        let iter = self.unfiltered_allocation_ids().iter().map( |&allocation_id| {
            (allocation_id, self.data.get_allocation( allocation_id ) )
        });

        dump_collation_from_iter( &self.data, iter, |line| {
            lines.push( line.to_owned() );
            let result: Result< (), () > = Ok(());
            result
        }).map_err( |_| Box::new( rhai::EvalAltResult::from( "failed to collate allocations" ) ) )?;

        lines.sort_unstable();

        let mut output = String::new();
        crate::exporter_flamegraph::lines_to_svg( lines, &mut output );

        Ok( output )
    }

    fn save_as_flamegraph( &mut self, env: &mut dyn Environment, path: String ) -> Result< Self, Box< rhai::EvalAltResult > > {
        let data = self.save_as_flamegraph_to_string()?;
        env.file_write( &path, FileKind::Svg, data.as_bytes() )?;
        Ok( self.clone() )
    }

    fn save_as_graph( &self, env: &mut dyn Environment, path: String ) -> Result< Self, Box< rhai::EvalAltResult > > {
        Graph::new().add( self.clone() ).save( env, path )?;
        Ok( self.clone() )
    }

    fn len( &mut self ) -> i64 {
        self.apply_filter();
        self.unfiltered_allocation_ids().len() as i64
    }

    fn filtered_ops( &mut self, mut callback: impl FnMut( AllocationId ) -> OpFilter ) -> Vec< OperationId > {
        self.apply_filter();
        let ids = self.unfiltered_allocation_ids();
        let mut ops = Vec::with_capacity( ids.len() );
        for &id in ids {
            let filter = callback( id );
            if filter == OpFilter::None {
                continue;
            }

            let allocation = self.data.get_allocation( id );
            ops.push( OperationId::new_allocation( id ) );

            if allocation.deallocation.is_some() && filter != OpFilter::OnlyAlloc {
                ops.push( OperationId::new_deallocation( id ) );
            }
        }

        ops.par_sort_by_key( |op| get_timestamp( &self.data, *op ) );
        ops
    }

    fn group_by_backtrace( &mut self ) -> AllocationGroupList {
        self.apply_filter();
        let mut groups = HashMap::new();
        for &id in self.unfiltered_allocation_ids() {
            let allocation = self.data.get_allocation( id );
            let group = groups.entry( allocation.backtrace ).or_insert_with( || AllocationGroup { allocation_ids: Vec::new() } );
            group.allocation_ids.push( id );
        }

        AllocationGroupList {
            data: self.data.clone(),
            groups: Arc::new( groups )
        }
    }
}

#[derive(Clone)]
struct AllocationGroup {
    allocation_ids: Vec< AllocationId >
}

#[derive(Clone)]
struct AllocationGroupList {
    data: DataRef,
    groups: Arc< HashMap< BacktraceId, AllocationGroup > >
}

impl std::fmt::Debug for AllocationGroupList {
    fn fmt( &self, fmt: &mut std::fmt::Formatter ) -> std::fmt::Result {
        write!( fmt, "AllocationGroupList" )
    }
}

impl AllocationGroupList {
    fn filter( &self, callback: impl Fn( &AllocationGroup ) -> bool + Send + Sync ) -> Self {
        let groups: Vec< _ > = self.groups.par_iter()
            .filter( |(_, group)| callback( group ) )
            .map( |(key, group)| (key.clone(), group.clone()) )
            .collect();

        Self {
            data: self.data.clone(),
            groups: Arc::new( groups.into_iter().collect() )
        }
    }

    fn only_all_leaked( &mut self ) -> AllocationGroupList {
        self.filter( |group| group.allocation_ids.par_iter().all( |&id| {
            let allocation = self.data.get_allocation( id );
            allocation.deallocation.is_none()
        }))
    }

    fn ungroup( &mut self ) -> AllocationList {
        let mut allocation_ids = Vec::new();
        for (_, group) in &*self.groups {
            allocation_ids.extend_from_slice( &group.allocation_ids );
        }
        allocation_ids.par_sort_by_key( |&id| {
            let allocation = self.data.get_allocation( id );
            (allocation.timestamp, id)
        });

        AllocationList {
            data: self.data.clone(),
            allocation_ids: Some( Arc::new( allocation_ids ) ),
            filter: None
        }
    }
}

#[derive(Clone)]
struct Graph {
    without_legend: bool,
    without_axes: bool,
    without_grid: bool,
    hide_empty: bool,
    trim_left: bool,
    trim_right: bool,
    extend_until: Option< Duration >,
    truncate_until: Option< Duration >,
    lists: Vec< AllocationList >,
    labels: Vec< Option< String > >,
    gradient: Option< Arc< colorgrad::Gradient > >,

    cached_datapoints: Option< Arc< (Vec< u64 >, Vec< Vec< (u64, u64) > >) > >
}

fn prepare_graph_datapoints( data: &Data, ops_for_list: &[Vec< OperationId >] ) -> (Vec< u64 >, Vec< Vec< (u64, u64) > >) {
    let timestamp_min = ops_for_list.iter().flat_map( |ops| ops.first() ).map( |op| get_timestamp( &data, *op ) ).min().unwrap_or( common::Timestamp::min() );
    let timestamp_max = ops_for_list.iter().flat_map( |ops| ops.last() ).map( |op| get_timestamp( &data, *op ) ).max().unwrap_or( common::Timestamp::min() );

    let mut xs = HashSet::new();
    let mut datapoints_for_ops = Vec::new();
    for ops in ops_for_list {
        if ops.is_empty() {
            datapoints_for_ops.push( Vec::new() );
            continue;
        }

        let datapoints: Vec< _ > = build_timeline( &data, timestamp_min, timestamp_max, ops ).into_iter().map( |point| {
            xs.insert( point.timestamp );
            (point.timestamp, point.memory_usage)
        }).collect();

        datapoints_for_ops.push( datapoints );
    }

    let mut xs: Vec< _ > = xs.into_iter().collect();
    xs.sort_unstable();

    for datapoints in &mut datapoints_for_ops {
        if datapoints.is_empty() {
            continue;
        }
        *datapoints = expand_datapoints( &xs, &datapoints );
    }

    for index in 0..xs.len() {
        let mut value = 0;
        for datapoints in datapoints_for_ops.iter_mut() {
            if datapoints.is_empty() {
                continue;
            }

            value += datapoints[ index ].1;
            datapoints[ index ].1 = value;
        }
    }

    (xs, datapoints_for_ops)
}

impl Graph {
    fn new() -> Self {
        Graph {
            without_legend: false,
            without_axes: false,
            without_grid: false,
            hide_empty: false,
            trim_left: false,
            trim_right: false,
            extend_until: None,
            truncate_until: None,
            lists: Vec::new(),
            labels: Vec::new(),
            gradient: None,

            cached_datapoints: None
        }
    }

    fn add_with_label( &mut self, label: String, list: AllocationList ) -> Self {
        let mut cloned = self.clone();
        cloned.lists.push( list );
        cloned.labels.push( Some( label ) );
        cloned.cached_datapoints = None;
        cloned
    }

    fn add( &mut self, list: AllocationList ) -> Self {
        let mut cloned = self.clone();
        cloned.lists.push( list );
        cloned.labels.push( None );
        cloned.cached_datapoints = None;
        cloned
    }

    fn only_non_empty_series( &mut self ) -> Self {
        let mut cloned = self.clone();
        cloned.hide_empty = true;
        cloned
    }

    fn trim( &mut self ) -> Self {
        let mut cloned = self.clone();
        cloned.trim_left = true;
        cloned.trim_right = true;
        cloned
    }

    fn trim_left( &mut self ) -> Self {
        let mut cloned = self.clone();
        cloned.trim_left = true;
        cloned
    }

    fn trim_right( &mut self ) -> Self {
        let mut cloned = self.clone();
        cloned.trim_right = true;
        cloned
    }

    fn extend_until( &mut self, offset: Duration ) -> Self {
        let mut cloned = self.clone();
        cloned.extend_until = Some( offset );
        cloned
    }

    fn truncate_until( &mut self, offset: Duration ) -> Self {
        let mut cloned = self.clone();
        cloned.truncate_until = Some( offset );
        cloned
    }

    fn without_legend( &mut self ) -> Self {
        let mut cloned = self.clone();
        cloned.without_legend = true;
        cloned
    }

    fn without_axes( &mut self ) -> Self {
        let mut cloned = self.clone();
        cloned.without_axes = true;
        cloned
    }

    fn without_grid( &mut self ) -> Self {
        let mut cloned = self.clone();
        cloned.without_grid = true;
        cloned
    }

    fn generate_ops( &mut self ) -> Result< Vec< Vec< OperationId > >, String > {
        let lists = &mut self.lists;
        if lists.is_empty() {
            return Err( format!( "no allocation lists given" ) );
        }

        let data = lists[ 0 ].data.clone();
        if !lists.iter().all( |list| list.data.id() == data.id() ) {
            return Err( format!( "not every allocation list given is from the same data file" ) );
        }

        let threshold = self.truncate_until.map( |offset| data.initial_timestamp + offset.0 ).unwrap_or( data.last_timestamp );

        let mut seen = HashSet::new();
        let ops_for_list: Vec< _ > = lists.iter_mut().map( |list|
            list.filtered_ops( |id| {
                if !seen.insert( id ) {
                    return OpFilter::None;
                }

                let allocation = data.get_allocation( id );
                if allocation.timestamp > threshold {
                    return OpFilter::None;
                }

                if let Some( ref deallocation ) = allocation.deallocation {
                    if deallocation.timestamp > threshold {
                        return OpFilter::OnlyAlloc;
                    }
                }

                OpFilter::Both
            })
        ).collect();

        Ok( ops_for_list )
    }

    fn with_gradient_color_scheme( &mut self, start: String, end: String ) -> Result< Self, Box< rhai::EvalAltResult > > {
        let mut cloned = self.clone();
        cloned.gradient = Some( Arc::new(
            colorgrad::CustomGradient::new()
                .html_colors( &[start.as_str(), end.as_str()] )
                .build().map_err( |err| {
                    error( format!( "failed to create a gradient: {}", err ) )
                })?
        ));

        return Ok( cloned );
    }

    fn save_to_string_impl( &self, xs: &[u64], datapoints_for_ops: &[Vec< (u64, u64) >], labels: &[Option< String >] ) -> Result< String, String > {
        let data = self.lists[ 0 ].data.clone();

        let mut max_usage = 0;
        for datapoints in datapoints_for_ops {
            for (_, value) in datapoints {
                max_usage = std::cmp::max( max_usage, *value );
            }
        }

        let mut x_min = xs.first().copied().unwrap_or( 0 );
        let mut x_max = xs.last().copied().unwrap_or( 0 );
        if let Some( truncate_until ) = self.truncate_until {
            x_max = std::cmp::min( x_max, (data.initial_timestamp + truncate_until.0).as_usecs() );
        }
        if let Some( extend_until ) = self.extend_until {
            x_max = std::cmp::max( x_max, (data.initial_timestamp + extend_until.0).as_usecs() );
        }

        if !self.trim_left {
            x_min = std::cmp::min( x_min, data.initial_timestamp.as_usecs() );
        }
        if !self.trim_right {
            x_max = std::cmp::max( x_max, data.last_timestamp.as_usecs() );
        }

        // This is a dirty hack, but it works.
        thread_local! {
            static SCALE_X: Cell< (u64, u64) > = Cell::new( (0, 0) );
            static SCALE_Y: Cell< (u64, u64) > = Cell::new( (0, 0) );
        }

        macro_rules! impl_ranged {
            ($kind:ty) => {
                impl Ranged for $kind {
                    type FormatOption = plotters::coord::ranged1d::NoDefaultFormatting;
                    type ValueType = u64;
                    fn map( &self, value: &Self::ValueType, limit: (i32, i32) ) -> i32 {
                        if self.0 == self.1 {
                            return (limit.1 - limit.0) / 2;
                        }

                        let screen_range = limit.1 - limit.0;
                        if screen_range == 0 {
                            return limit.1;
                        }

                        let data_range = self.1 - self.0;
                        let data_offset = value - self.0;
                        let data_relative_position = data_offset as f64 / data_range as f64;

                        limit.0 + (screen_range as f64 * data_relative_position + 1e-3).floor() as i32
                    }

                    fn key_points< Hint: plotters::coord::ranged1d::KeyPointHint >( &self, hint: Hint ) -> Vec< Self::ValueType > {
                        gen_keypoints( (self.0, self.1), hint.max_num_points() )
                    }

                    fn range( &self ) -> std::ops::Range< Self::ValueType > {
                        self.0..self.1
                    }
                }
            }
        }

        struct SizeRange( u64, u64 );

        impl plotters::coord::ranged1d::ValueFormatter< u64 > for SizeRange {
            fn format( value: &u64 ) -> String {
                SCALE_Y.with( |cell| {
                    let (min, max) = cell.get();

                    if max < 1024 {
                        format!( "{}", value )
                    } else {
                        let (unit, multiplier) = {
                            if max < 1024 * 1024 {
                                ("KB", 1024)
                            } else {
                                ("MB", 1024 * 1024)
                            }
                        };

                        if max - min <= (10 * multiplier) {
                            format!( "{:.02} {}", *value as f64 / multiplier as f64, unit )
                        } else if max - min <= (100 * multiplier) {
                            format!( "{:.01} {}", *value as f64 / multiplier as f64, unit )
                        } else {
                            format!( "{} {}", value / multiplier, unit )
                        }
                    }
                })
            }
        }

        impl_ranged!( SizeRange );

        struct TimeRange( u64, u64 );

        impl plotters::coord::ranged1d::ValueFormatter< u64 > for TimeRange {
            fn format( value: &u64 ) -> String {
                use chrono::prelude::*;

                SCALE_X.with( |cell| {
                    let (min, max) = cell.get();
                    debug_assert!( *value >= min );

                    let start = to_chrono( min );
                    let end = to_chrono( max );
                    let ts = to_chrono( *value );
                    if start.year() == end.year() && start.month() == end.month() && start.day() == end.day() {
                        format!( "{:02}:{:02}:{:02}", ts.hour(), ts.minute(), ts.second() )
                    } else if start.year() == end.year() && start.month() == end.month() {
                        format!( "{:02} {:02}:{:02}:{:02}", ts.day(), ts.hour(), ts.minute(), ts.second() )
                    } else if start.year() == end.year() {
                        format!( "{:02}-{:02} {:02}:{:02}:{:02}", ts.month(), ts.day(), ts.hour(), ts.minute(), ts.second() )
                    } else {
                        format!( "{}-{:02}-{:02} {:02}:{:02}:{:02}", ts.year(), ts.month(), ts.day(), ts.hour(), ts.minute(), ts.second() )
                    }
                })
            }
        }

        impl_ranged!( TimeRange );

        struct TimeRangeOffset( u64, u64 );

        impl plotters::coord::ranged1d::ValueFormatter< u64 > for TimeRangeOffset {
            fn format( value: &u64 ) -> String {
                SCALE_X.with( |cell| {
                    let (min, _max) = cell.get();
                    debug_assert!( *value >= min );
                    let relative = *value - min;
                    let relative_s = relative / 1_000_000;

                    if relative == 0 {
                        format!( "0" )
                    } else if relative < 1_000 {
                        format!( "+{}us", relative )
                    } else if relative < 1_000_000 {
                        format!( "+{}ms", relative / 1_000 )
                    } else if relative < 60_000_000 {
                        format!( "+{}s", relative / 1_000_000 )
                    } else {
                        let rh = relative_s / 3600;
                        let rm = (relative_s - rh * 3600) / 60;
                        let rs = relative_s - rh * 3600 - rm * 60;
                        return format!( "+{:02}:{:02}:{:02}", rh, rm, rs );
                    }
                })
            }
        }

        impl_ranged!( TimeRangeOffset );

        SCALE_X.with( |cell| cell.set( (x_min, x_max + 1) ) );
        SCALE_Y.with( |cell| cell.set( (0, (max_usage + 1) as u64) ) );

        let mut output = String::new();
        use plotters::prelude::*;
        let root = SVGBackend::with_string( &mut output, (1024, 768) ).into_drawing_area();
        root.fill( &WHITE ).map_err( |error| format!( "failed to fill the graph with white: {}", error ) )?;

        let mut chart = ChartBuilder::on( &root );
        let mut chart = &mut chart;
        if !self.without_axes {
            chart = chart
                .margin( (1).percent() )
                .set_label_area_size( LabelAreaPosition::Left, 70 )
                .margin_right( 50 )
                .set_label_area_size( LabelAreaPosition::Bottom, 60 )
                .set_label_area_size( LabelAreaPosition::Top, 60 )
        };

        let mut chart = chart.build_cartesian_2d(
                TimeRange( x_min, x_max + 1 ),
                SizeRange( 0, (max_usage + 1) as u64 )
            )
            .map_err( |error| format!( "failed to construct the chart builder: {}", error ) )?
            .set_secondary_coord(
                TimeRangeOffset( x_min, x_max + 1 ),
                SizeRange( 0, (max_usage + 1) as u64 )
            );

        let mut colors = Vec::new();
        if let Some( ref gradient ) = self.gradient {
            let step = 1.0 / (std::cmp::max( 1, datapoints_for_ops.len() ) - 1) as f64;
            for index in 0..datapoints_for_ops.len() {
                let position = index as f64 * step;
                let color = gradient.at( position );
                let color_rgb = color.rgba_u8();
                colors.push( RGBColor( color_rgb.0, color_rgb.1, color_rgb.2 ).to_rgba().mix( color.alpha() ) );
            }
        } else {
            for index in 0..datapoints_for_ops.len() {
                colors.push( Palette99::pick( index ).to_rgba() );
            }
        }

        for ((datapoints, label), color) in datapoints_for_ops.iter().zip( labels.iter() ).rev().zip( colors.into_iter().rev() ) {
            let series = chart.draw_series(
                AreaSeries::new(
                    datapoints.iter().map( |&(x, y)| {
                        (x, y as u64)
                    }).chain( std::iter::once((
                        x_max,
                        datapoints.last().copied().map( |(_, y)| y ).unwrap_or( 0 )
                    ))),
                    0_u64,
                    color,
                ).border_style( color.stroke_width( 1 ) ),
            ).map_err( |error| format!( "failed to draw a series: {}", error ) )?;

            if let Some( label ) = label {
                if datapoints.is_empty() && self.hide_empty || self.without_legend {
                    continue;
                }

                series
                    .label( label )
                    .legend( move |(x, y)| Rectangle::new( [(x, y - 5), (x + 10, y + 5)], color.filled() ) );
            }
        }

        let mut mesh = chart.configure_mesh();
        let mut mesh = &mut mesh;
        if !self.without_axes {
            mesh = mesh.x_desc( "Time" ).y_desc( "Memory usage" );
        }

        if self.without_grid {
            mesh = mesh.disable_mesh();
        }

        mesh.draw().map_err( |error| format!( "failed to draw the mesh: {}", error ) )?;

        if !self.without_axes {
            chart
                .configure_secondary_axes()
                .draw()
                .map_err( |error| format!( "failed to draw the secondary axes: {}", error ) )?;
        }

        if labels.iter().any( |label| label.is_some() ) && !self.without_legend {
            chart
                .configure_series_labels()
                .background_style( &WHITE.mix( 0.75 ) )
                .border_style( &BLACK )
                .position( SeriesLabelPosition::UpperLeft )
                .draw()
                .map_err( |error| format!( "failed to draw the legend: {}", error ) )?;
        }

        root.present().map_err( |error| format!( "failed to present the graph: {}", error ) )?;
        std::mem::drop( chart );
        std::mem::drop( root );

        Ok( output )
    }

    fn save_to_string( &mut self ) -> Result< String, Box< rhai::EvalAltResult > > {
        (|| {
            if self.cached_datapoints.is_none() {
                let ops_for_list = self.generate_ops()?;
                let (xs, datapoints_for_ops) = prepare_graph_datapoints( &self.lists[ 0 ].data, &ops_for_list );
                self.cached_datapoints = Some( Arc::new( (xs, datapoints_for_ops) ) );
            }

            let cached = self.cached_datapoints.as_ref().unwrap();
            self.save_to_string_impl( &cached.0, &cached.1, &self.labels )
        }.map_err( |error| {
            Box::new( rhai::EvalAltResult::from( format!( "failed to generate a graph: {}", error ) ) )
        }))()
    }

    fn save( &mut self, env: &mut dyn Environment, path: String ) -> Result< Self, Box< rhai::EvalAltResult > > {
        let data = self.save_to_string()?;
        env.file_write( &path, FileKind::Svg, data.as_bytes() )?;
        Ok( self.clone() )
    }

    fn save_each_series_as_graph( &mut self, env: &mut dyn Environment, mut path: String ) -> Result< Self, Box< rhai::EvalAltResult > > {
        env.mkdir_p( &path )?;
        if path == "." {
            path = "".into();
        } else if !path.ends_with( '/' ) {
            path.push( '/' );
        }

        let ops_for_list = self.generate_ops()?;
        for (index, (ops, label)) in ops_for_list.into_iter().zip( self.labels.iter() ).enumerate() {
            let (xs, datapoints_for_ops) = prepare_graph_datapoints( &self.lists[ 0 ].data, &[ops] );
            let data = self.save_to_string_impl( &xs, &datapoints_for_ops, &[label.clone()] )?;

            let file_path =
                if let Some( label ) = label {
                    format!( "{}{}.svg", path, label )
                } else {
                    format!( "{}Series #{}.svg", path, index )
                };

            env.file_write( &file_path, FileKind::Svg, data.as_bytes() )?;
        }

        Ok( self.clone() )
    }

    fn save_each_series_as_flamegraph( &mut self, env: &mut dyn Environment, mut path: String ) -> Result< Self, Box< rhai::EvalAltResult > > {
        env.mkdir_p( &path )?;
        if path == "." {
            path = "".into();
        } else if !path.ends_with( '/' ) {
            path.push( '/' );
        }

        let ops_for_list = self.generate_ops()?;
        for (index, ((list, ops), label)) in self.lists.iter().zip( ops_for_list ).zip( self.labels.iter() ).enumerate() {
            let ids: HashSet< _ > = ops.into_iter().map( |op| op.id() ).collect();
            let mut list = AllocationList {
                data: list.data.clone(),
                allocation_ids: Some( Arc::new( ids.into_iter().collect() ) ),
                filter: None
            };

            let file_path =
                if let Some( label ) = label {
                    format!( "{}{}.svg", path, label )
                } else {
                    format!( "{}Series #{}.svg", path, index )
                };

            list.save_as_flamegraph( env, file_path )?;
        }
        Ok( self.clone() )
    }
}

fn load( path: String ) -> Result< Arc< Data >, Box< rhai::EvalAltResult > > {
    info!( "Loading {:?}...", path );
    let fp = File::open( &path )
        .map_err( |error| rhai::EvalAltResult::from( format!( "failed to open '{}': {}", path, error ) ) )
        .map_err( Box::new )?;

    let debug_symbols: &[PathBuf] = &[];
    let data = Loader::load_from_stream( fp, debug_symbols )
        .map_err( |error| rhai::EvalAltResult::from( format!( "failed to load '{}': {}", path, error ) ) )
        .map_err( Box::new )?;

    Ok( Arc::new( data ) )
}

fn uses_same_list( lhs: &AllocationList, rhs: &AllocationList ) -> bool {
    match (lhs.allocation_ids.as_ref(), rhs.allocation_ids.as_ref()) {
        (None, None) => true,
        (Some( lhs ), Some( rhs )) => {
            Arc::ptr_eq( lhs, rhs )
        },
        _ => false
    }
}

fn merge_allocations( mut lhs: AllocationList, mut rhs: AllocationList ) -> Result< AllocationList, Box< rhai::EvalAltResult > > {
    if lhs.data.id != rhs.data.id {
        return Err( Box::new( rhai::EvalAltResult::from( "allocation list don't come from the same data file" ) ) );
    }

    if uses_same_list( &lhs, &rhs ) {
        let filter = match (lhs.filter.as_ref(), rhs.filter.as_ref()) {
            (Some( lhs ), Some( rhs )) => Some( Filter::Or( Box::new( lhs.clone() ), Box::new( rhs.clone() ) ) ),
            _ => None
        };

        Ok( AllocationList {
            data: lhs.data.clone(),
            allocation_ids: lhs.allocation_ids.clone(),
            filter
        })
    } else {
        lhs.apply_filter();
        rhs.apply_filter();

        let mut set: HashSet< AllocationId > = HashSet::new();
        set.extend( lhs.unfiltered_allocation_ids() );
        set.extend( rhs.unfiltered_allocation_ids() );

        let ids: Vec< _ > = lhs.data.sorted_by_timestamp.par_iter().copied().filter( |id| set.contains( &id ) ).collect();
        Ok( AllocationList {
            data: lhs.data.clone(),
            allocation_ids: Some( Arc::new( ids ) ),
            filter: None
        })
    }
}

fn substract_allocations( lhs: AllocationList, mut rhs: AllocationList ) -> Result< AllocationList, Box< rhai::EvalAltResult > > {
    if lhs.data.id != rhs.data.id {
        return Err( Box::new( rhai::EvalAltResult::from( "allocation list don't come from the same data file" ) ) );
    }

    if uses_same_list( &lhs, &rhs ) {
        let filter = match (lhs.filter.as_ref(), rhs.filter.as_ref()) {
            (_, None) => {
                return Ok( AllocationList {
                    data: lhs.data.clone(),
                    allocation_ids: Some( Arc::new( Vec::new() ) ),
                    filter: None
                });
            },
            (None, Some( rhs )) => Some(
                Filter::Not(
                    Box::new( rhs.clone() )
                )
            ),
            (Some( lhs ), Some( rhs )) => Some(
                Filter::And(
                    Box::new( lhs.clone() ),
                    Box::new( Filter::Not(
                        Box::new( rhs.clone() )
                    ))
                )
            )
        };

        Ok( AllocationList {
            data: lhs.data.clone(),
            allocation_ids: lhs.allocation_ids.clone(),
            filter
        })
    } else {
        rhs.apply_filter();

        let mut set: HashSet< AllocationId > = HashSet::new();
        set.extend( rhs.unfiltered_allocation_ids() );

        let ids: Vec< _ > = lhs.filtered_allocation_ids().filter( |id| !set.contains( id ) ).collect();
        Ok( AllocationList {
            data: lhs.data.clone(),
            allocation_ids: Some( Arc::new( ids ) ),
            filter: None
        })
    }
}

fn intersect_allocations( lhs: AllocationList, mut rhs: AllocationList ) -> Result< AllocationList, Box< rhai::EvalAltResult > > {
    if lhs.data.id != rhs.data.id {
        return Err( Box::new( rhai::EvalAltResult::from( "allocation list don't come from the same data file" ) ) );
    }

    if uses_same_list( &lhs, &rhs ) {
        let filter = match (lhs.filter.as_ref(), rhs.filter.as_ref()) {
            (None, None) => None,
            (Some( lhs ), None) => Some( lhs.clone() ),
            (None, Some( rhs )) => Some( rhs.clone() ),
            (Some( lhs ), Some( rhs )) => Some( Filter::And( Box::new( lhs.clone() ), Box::new( rhs.clone() ) ) )
        };

        Ok( AllocationList {
            data: lhs.data.clone(),
            allocation_ids: lhs.allocation_ids.clone(),
            filter
        })
    } else {
        rhs.apply_filter();

        let mut set: HashSet< AllocationId > = HashSet::new();
        set.extend( rhs.unfiltered_allocation_ids() );

        let ids: Vec< _ > = lhs.filtered_allocation_ids().filter( |id| set.contains( id ) ).collect();
        Ok( AllocationList {
            data: lhs.data.clone(),
            allocation_ids: Some( Arc::new( ids ) ),
            filter: None
        })
    }
}

pub fn error( message: impl Into< String > ) -> Box< rhai::EvalAltResult > {
    Box::new( rhai::EvalAltResult::from( message.into() ) )
}

#[derive(Copy, Clone)]
pub enum FileKind {
    Svg
}

pub struct Engine {
    inner: rhai::Engine
}

#[derive(Default)]
pub struct EngineArgs {
    pub argv: Vec< String >,
    pub data: Option< Arc< Data > >,
    pub allocation_ids: Option< Arc< Vec< AllocationId > > >
}

pub trait Environment {
    fn println( &mut self, message: &str );
    fn mkdir_p( &mut self, path: &str ) -> Result< (), Box< rhai::EvalAltResult > >;
    fn chdir( &mut self, path: &str ) -> Result< (), Box< rhai::EvalAltResult > >;
    fn file_write( &mut self, path: &str, kind: FileKind, contents: &[u8] ) -> Result< (), Box< rhai::EvalAltResult > >;
    fn exit( &mut self, errorcode: Option< i32 > ) -> Result< (), Box< rhai::EvalAltResult > > {
        Err( Box::new( rhai::EvalAltResult::Return( (errorcode.unwrap_or( 0 ) as i64).into(), rhai::Position::NONE ) ) )
    }
    fn load( &mut self, _path: String ) -> Result< Arc< Data >, Box< rhai::EvalAltResult > > {
        Err( error( "unsupported in this environment" ) )
    }
}

#[derive(Default)]
pub struct NativeEnvironment {}

impl Environment for NativeEnvironment {
    fn println( &mut self, message: &str ) {
        println!( "{}", message );
    }

    fn mkdir_p( &mut self, path: &str ) -> Result< (), Box< rhai::EvalAltResult > > {
        std::fs::create_dir_all( path ).map_err( |error| format!( "failed to create '{}': {}", path, error ).into() ).map_err( Box::new )
    }

    fn chdir( &mut self, path: &str ) -> Result< (), Box< rhai::EvalAltResult > > {
        std::env::set_current_dir( path ).map_err( |error| format!( "failed to chdir to '{}': {}", path, error ).into() ).map_err( Box::new )
    }

    fn file_write( &mut self, path: &str, _kind: FileKind, contents: &[u8] ) -> Result< (), Box< rhai::EvalAltResult > > {
        use std::io::Write;

        let mut fp = File::create( &path )
            .map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to create {:?}: {}", path, error ) ) ) )?;

        fp.write_all( contents )
            .map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to write to {:?}: {}", path, error ) ) ) )?;

        Ok(())
    }

    fn exit( &mut self, errorcode: Option< i32 > ) -> Result< (), Box< rhai::EvalAltResult > > {
        std::process::exit( errorcode.unwrap_or( 0 ) );
    }

    fn load( &mut self, path: String ) -> Result< Arc< Data >, Box< rhai::EvalAltResult > > {
        load( path )
    }
}

fn to_string( value: rhai::plugin::Dynamic ) -> String {
    if value.is::< String >() {
        value.cast::< String >()
    } else if value.is::< i64 >() {
        value.cast::< i64 >().to_string()
    } else if value.is::< u64 >() {
        value.cast::< u64 >().to_string()
    } else if value.is::< bool >() {
        value.cast::< bool >().to_string()
    } else if value.is::< f64 >() {
        value.cast::< f64 >().to_string()
    } else if value.is::< Duration >() {
        value.cast::< Duration >().decompose().to_string()
    } else {
        value.type_name().into()
    }
}

fn format( fmt: &str, args: &[&str] ) -> Result< String, Box< rhai::EvalAltResult > > {
    let mut output = String::with_capacity( fmt.len() );
    let mut tmp = String::new();
    let mut in_interpolation = false;
    let mut current_arg = 0;
    for ch in fmt.chars() {
        if in_interpolation {
            if tmp.is_empty() && ch == '{' {
                in_interpolation = false;
                output.push( ch );
                continue;
            }
            if ch == '}' {
                in_interpolation = false;
                if tmp.is_empty() {
                    if current_arg >= args.len() {
                        return Err( error( "too many positional arguments in the format string" ) );
                    }
                    output.push_str( args[ current_arg ] );
                    current_arg += 1;
                } else {
                    let position: Result< usize, _ > = tmp.parse();
                    if let Ok( position ) = position {
                        tmp.clear();
                        if position >= args.len() {
                            return Err( error( format!( "invalid reference to positional argument {}", position ) ) );
                        }
                        output.push_str( args[ position ] );
                    } else {
                        return Err( error( format!( "malformed positional argument \"{}\"", tmp ) ) );
                    }
                }
                continue;
            }
            tmp.push( ch );
        } else {
            if ch == '{' {
                in_interpolation = true;
                continue;
            }
            output.push( ch );
        }
    }

    if in_interpolation {
        return Err( error( "malformed format string" ) );
    }

    Ok( output )
}

impl Engine {
    pub fn new( env: Arc< Mutex< dyn Environment > >, args: EngineArgs ) -> Self {
        use rhai::packages::Package;

        let mut engine = rhai::Engine::new_raw();
        engine.register_global_module( rhai::packages::ArithmeticPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::BasicArrayPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::BasicFnPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::BasicIteratorPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::BasicMapPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::BasicMathPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::BasicStringPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::LogicPackage::new().as_shared_module() );
        engine.register_global_module( rhai::packages::MoreStringPackage::new().as_shared_module() );

        let argv = args.argv;

        // Utility functions.
        engine.register_fn( "dirname", dirname );
        engine.register_fn( "h", |value: i64| Duration::from_secs( value as u64 * 3600 ) );
        engine.register_fn( "h", |value: f64| Duration::from_usecs( (value * 3600.0 * 1_000_000.0) as u64 ) );
        engine.register_fn( "m", |value: i64| Duration::from_secs( value as u64 * 60 ) );
        engine.register_fn( "m", |value: f64| Duration::from_usecs( (value * 60.0 * 1_000_000.0) as u64 ) );
        engine.register_fn( "s", |value: i64| Duration::from_secs( value as u64 ) );
        engine.register_fn( "s", |value: f64| Duration::from_secs( (value * 1_000_000.0) as u64 ) );
        engine.register_fn( "ms", |value: i64| Duration::from_msecs( value as u64 ) );
        engine.register_fn( "ms", |value: f64| Duration::from_usecs( (value * 1_000.0) as u64 ) );
        engine.register_fn( "us", |value: i64| Duration::from_usecs( value as u64 ) );
        engine.register_fn( "us", |value: f64| Duration::from_usecs( value as u64 ) );
        engine.register_fn( "*", |lhs: Duration, rhs: i64| -> Duration { Duration( lhs.0 * rhs as f64 ) } );
        engine.register_fn( "*", |lhs: i64, rhs: Duration| -> Duration { Duration( rhs.0 * lhs as f64 ) } );
        engine.register_fn( "*", |lhs: Duration, rhs: f64| -> Duration { Duration( lhs.0 * rhs as f64 ) } );
        engine.register_fn( "*", |lhs: f64, rhs: Duration| -> Duration { Duration( rhs.0 * lhs as f64 ) } );
        engine.register_fn( "+", |lhs: Duration, rhs: Duration| -> Duration { Duration( lhs.0 + rhs.0 ) } );
        engine.register_fn( "-", |lhs: Duration, rhs: Duration| -> Duration { Duration( lhs.0 - rhs.0 ) } );
        engine.register_fn( "kb", |value: i64| value * 1000 );
        engine.register_fn( "mb", |value: i64| value * 1000 * 1000 );
        engine.register_fn( "gb", |value: i64| value * 1000 * 1000 * 1000 );
        engine.register_fn( "info", |message: &str| info!( "{}", message ) );
        engine.register_type::< Duration >();
        engine.register_fn( "argv", move || -> rhai::Array {
            argv.iter().cloned().map( rhai::Dynamic::from ).collect()
        });

        {
            let env = env.clone();
            engine.register_result_fn( "mkdir_p", move |path: &str| env.lock().mkdir_p( path ) );
        }
        {
            let env = env.clone();
            engine.register_result_fn( "chdir", move |path: &str| env.lock().chdir( path ) );
        }
        {
            let env = env.clone();
            engine.register_result_fn( "exit", move |errorcode: i64| env.lock().exit( Some( errorcode as i32 ) ) );
        }
        {
            let env = env.clone();
            engine.register_result_fn( "exit", move || env.lock().exit( None ) );
        }

        // DSL functions.
        engine.register_type::< DataRef >();
        engine.register_type::< AllocationList >();
        engine.register_type::< AllocationGroupList >();
        engine.register_type::< Graph >();
        engine.register_result_fn( "+", merge_allocations );
        engine.register_result_fn( "-", substract_allocations );
        engine.register_result_fn( "&", intersect_allocations );
        engine.register_fn( "graph", Graph::new );
        engine.register_fn( "add", Graph::add );
        engine.register_fn( "add", Graph::add_with_label );
        engine.register_fn( "trim_left", Graph::trim_left );
        engine.register_fn( "trim_right", Graph::trim_right );
        engine.register_fn( "trim", Graph::trim );
        engine.register_fn( "extend_until", Graph::extend_until );
        engine.register_fn( "truncate_until", Graph::truncate_until );
        engine.register_fn( "only_non_empty_series", Graph::only_non_empty_series );
        engine.register_fn( "without_legend", Graph::without_legend );
        engine.register_fn( "without_axes", Graph::without_axes );
        engine.register_fn( "without_grid", Graph::without_grid );
        engine.register_result_fn( "with_gradient_color_scheme", Graph::with_gradient_color_scheme );
        engine.register_fn( "allocations", DataRef::allocations );
        engine.register_fn( "runtime", |data: &mut DataRef| Duration( data.0.last_timestamp - data.0.initial_timestamp ) );

        fn set_max< T >( target: &mut Option< T >, value: T ) where T: PartialOrd {
            if let Some( target ) = target.as_mut() {
                if *target < value {
                    *target = value;
                }
            } else {
                *target = Some( value );
            }
        }

        fn set_min< T >( target: &mut Option< T >, value: T ) where T: PartialOrd {
            if let Some( target ) = target.as_mut() {
                if *target > value {
                    *target = value;
                }
            } else {
                *target = Some( value );
            }
        }

        engine.register_fn( "len", AllocationList::len );
        engine.register_result_fn( "only_passing_through_function", |list: &mut AllocationList, regex: String| {
            let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
            Ok( list.add_filter_once( |filter| filter.only_passing_through_function.is_some(), |filter|
                filter.only_passing_through_function = Some( regex )
            ))
        });
        engine.register_result_fn( "only_not_passing_through_function", |list: &mut AllocationList, regex: String| {
            let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
            Ok( list.add_filter_once( |filter| filter.only_not_passing_through_function.is_some(), |filter|
                filter.only_not_passing_through_function = Some( regex )
            ))
        });
        engine.register_result_fn( "only_passing_through_source", |list: &mut AllocationList, regex: String| {
            let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
            Ok( list.add_filter_once( |filter| filter.only_passing_through_source.is_some(), |filter|
                filter.only_passing_through_source = Some( regex )
            ))
        });
        engine.register_result_fn( "only_not_passing_through_source", |list: &mut AllocationList, regex: String| {
            let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
            Ok( list.add_filter_once( |filter| filter.only_not_passing_through_source.is_some(), |filter|
                filter.only_not_passing_through_source = Some( regex )
            ))
        });
        engine.register_result_fn( "only_matching_backtraces", |list: &mut AllocationList, ids: rhai::Array| {
            let mut set = HashSet::new();
            for id in ids {
                if let Some( id ) = id.try_cast::< i64 >() {
                    set.insert( BacktraceId::new( id as u32 ) );
                } else {
                    return Err( error( "expected an array of numbers" ) );
                }
            }

            if set.len() == 1 && list.allocation_ids.is_none() {
                let id = set.into_iter().next().unwrap();
                return Ok( AllocationList {
                    data: list.data.clone(),
                    allocation_ids: Some( Arc::new( list.data.get_allocation_ids_by_backtrace( id ).to_owned() ) ),
                    filter: list.filter.clone()
                });
            }

            Ok( list.add_filter( |filter| {
                if let Some( ref mut existing ) = filter.only_matching_backtraces {
                    *existing = existing.intersection( &set ).copied().collect();
                } else {
                    filter.only_matching_backtraces = Some( set );
                }
            }) )
        });

        macro_rules! register_filter {
            ($setter:ident, $name:ident, $src_ty:ty => $dst_ty:ty) => {
                engine.register_fn( stringify!( $name ), |list: &mut AllocationList, value: $src_ty|
                    list.add_filter( |filter| $setter( &mut filter.$name, value as $dst_ty ) )
                );
            };

            ($name:ident, bool) => {
                engine.register_fn( stringify!( $name ), |list: &mut AllocationList|
                    list.add_filter( |filter| filter.$name = true )
                );
            };

            ($setter:ident, $name:ident, $ty:ty) => {
                engine.register_fn( stringify!( $name ), |list: &mut AllocationList, value: $ty|
                    list.add_filter( |filter| $setter( &mut filter.$name, value as $ty ) )
                );
            };
        }

        register_filter!( set_max, only_backtrace_length_at_least, i64 => usize );
        register_filter!( set_min, only_backtrace_length_at_most, i64 => usize );
        register_filter!( set_max, only_larger_or_equal, i64 => u64 );
        register_filter!( set_min, only_smaller_or_equal, i64 => u64 );
        register_filter!( set_max, only_larger, i64 => u64 );
        register_filter!( set_min, only_smaller, i64 => u64 );
        register_filter!( set_max, only_address_at_least, i64 => u64 );
        register_filter!( set_min, only_address_at_most, i64 => u64 );

        register_filter!( set_max, only_allocated_after_at_least, Duration );
        register_filter!( set_min, only_allocated_until_at_most, Duration );
        register_filter!( set_max, only_deallocated_after_at_least, Duration );
        register_filter!( set_min, only_deallocated_until_at_most, Duration );
        register_filter!( set_max, only_not_deallocated_after_at_least, Duration );
        register_filter!( set_min, only_not_deallocated_until_at_most, Duration );
        register_filter!( set_max, only_alive_for_at_least, Duration );
        register_filter!( set_min, only_alive_for_at_most, Duration );

        register_filter!( set_max, only_leaked_or_deallocated_after, Duration );

        register_filter!( set_max, only_first_size_larger_or_equal, i64 => u64 );
        register_filter!( set_min, only_first_size_smaller_or_equal, i64 => u64 );
        register_filter!( set_max, only_first_size_larger, i64 => u64 );
        register_filter!( set_min, only_first_size_smaller, i64 => u64 );
        register_filter!( set_max, only_last_size_larger_or_equal, i64 => u64 );
        register_filter!( set_min, only_last_size_smaller_or_equal, i64 => u64 );
        register_filter!( set_max, only_last_size_larger, i64 => u64 );
        register_filter!( set_min, only_last_size_smaller, i64 => u64 );
        register_filter!( set_max, only_chain_length_at_least, i64 => u32 );
        register_filter!( set_min, only_chain_length_at_most, i64 => u32 );
        register_filter!( set_max, only_chain_alive_for_at_least, Duration );
        register_filter!( set_min, only_chain_alive_for_at_most, Duration );

        register_filter!( set_max, only_group_allocations_at_least, i64 => usize );
        register_filter!( set_min, only_group_allocations_at_most, i64 => usize );
        register_filter!( set_max, only_group_interval_at_least, Duration );
        register_filter!( set_min, only_group_interval_at_most, Duration );

        engine.register_fn( "only_group_leaked_allocations_at_least", |list: &mut AllocationList, value: f64| {
            list.add_filter_once( |filter| filter.only_group_leaked_allocations_at_least.is_some(), |filter|
                filter.only_group_leaked_allocations_at_least = Some( NumberOrFractionOfTotal::Fraction( value ) )
            )
        });
        engine.register_fn( "only_group_leaked_allocations_at_least", |list: &mut AllocationList, value: i64| {
            list.add_filter_once( |filter| filter.only_group_leaked_allocations_at_least.is_some(), |filter|
                filter.only_group_leaked_allocations_at_least = Some( NumberOrFractionOfTotal::Number( value as u64 ) )
            )
        });
        engine.register_fn( "only_group_leaked_allocations_at_most", |list: &mut AllocationList, value: f64| {
            list.add_filter_once( |filter| filter.only_group_leaked_allocations_at_most.is_some(), |filter|
                filter.only_group_leaked_allocations_at_most = Some( NumberOrFractionOfTotal::Fraction( value ) )
            )
        });
        engine.register_fn( "only_group_leaked_allocations_at_most", |list: &mut AllocationList, value: i64| {
            list.add_filter_once( |filter| filter.only_group_leaked_allocations_at_most.is_some(), |filter|
                filter.only_group_leaked_allocations_at_most = Some( NumberOrFractionOfTotal::Number( value as u64 ) )
            )
        });

        register_filter!( only_leaked, bool );
        register_filter!( only_temporary, bool );
        register_filter!( only_ptmalloc_mmaped, bool );
        register_filter!( only_ptmalloc_not_mmaped, bool );
        register_filter!( only_ptmalloc_from_main_arena, bool );
        register_filter!( only_ptmalloc_not_from_main_arena, bool );

        engine.register_fn( "only_with_marker", |list: &mut AllocationList, value: i64| {
            list.add_filter_once( |filter| filter.only_with_marker.is_some(), |filter|
                filter.only_with_marker = Some( value as u32 )
            )
        });

        engine.register_fn( "group_by_backtrace", AllocationList::group_by_backtrace );

        engine.register_fn( "only_all_leaked", AllocationGroupList::only_all_leaked );
        engine.register_fn( "ungroup", AllocationGroupList::ungroup );

        let graph_counter = Arc::new( AtomicUsize::new( 1 ) );
        let flamegraph_counter = Arc::new( AtomicUsize::new( 1 ) );

        fn get_counter( graph_counter: &AtomicUsize ) -> usize {
            graph_counter.fetch_add( 1, std::sync::atomic::Ordering::SeqCst )
        }

        {
            let data = args.data.clone();
            engine.register_result_fn( "data", move || {
                if let Some( ref data ) = data {
                    Ok( DataRef( data.clone() ) )
                } else {
                    Err( error( "no globally loaded data file" ) )
                }
            });
        }

        {
            let data = args.data.clone();
            let allocation_ids = args.allocation_ids.clone();
            engine.register_result_fn( "allocations", move || {
                if let Some( ref data ) = data {
                    Ok( AllocationList {
                        data: DataRef( data.clone() ),
                        allocation_ids: allocation_ids.clone(),
                        filter: None
                    })
                } else {
                    Err( error( "no globally loaded allocations" ) )
                }
            });
        }

        {
            let env = env.clone();
            engine.register_result_fn( "load", move |path: String| Ok( DataRef( env.lock().load( path )? ) ) );
        }

        {
            let env = env.clone();
            engine.register_result_fn(
                "save",
                move |graph: &mut Graph, path: String| Graph::save( graph, &mut *env.lock(), path )
            );
        }
        {
            let env = env.clone();
            let graph_counter = graph_counter.clone();
            engine.register_result_fn(
                "save",
                move |graph: &mut Graph| Graph::save( graph, &mut *env.lock(), format!( "Graph #{}.svg", get_counter( &graph_counter ) ) )
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_each_series_as_graph",
                move |graph: &mut Graph, path: String| Graph::save_each_series_as_graph( graph, &mut *env.lock(), path )
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_each_series_as_graph",
                move |graph: &mut Graph| Graph::save_each_series_as_graph( graph, &mut *env.lock(), ".".into() )
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_each_series_as_flamegraph",
                move |graph: &mut Graph, path: String| Graph::save_each_series_as_flamegraph( graph, &mut *env.lock(), path )
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_each_series_as_flamegraph",
                move |graph: &mut Graph| Graph::save_each_series_as_flamegraph( graph, &mut *env.lock(), ".".into() )
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_as_flamegraph",
                move |list: &mut AllocationList, path: String| AllocationList::save_as_flamegraph( list, &mut *env.lock(), path )
            );
        }
        {
            let env = env.clone();
            let flamegraph_counter = flamegraph_counter.clone();
            engine.register_result_fn(
                "save_as_flamegraph",
                move |list: &mut AllocationList| AllocationList::save_as_flamegraph( list, &mut *env.lock(), format!( "Flamegraph #{}.svg", get_counter( &flamegraph_counter ) ) )
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_as_graph",
                move |list: &mut AllocationList, path: String| AllocationList::save_as_graph( list, &mut *env.lock(), path )
            );
        }
        {
            let env = env.clone();
            let graph_counter = graph_counter.clone();
            engine.register_result_fn(
                "save_as_graph",
                move |list: &mut AllocationList| AllocationList::save_as_graph( list, &mut *env.lock(), format!( "Graph #{}.svg", get_counter( &graph_counter ) ) )
            );
        }

        {
            let env = env.clone();
            engine.register_fn(
                "println",
                move || {
                    env.lock().println( "" );
                }
            );
        }

        {
            let env = env.clone();
            engine.register_fn(
                "println",
                move |a0: rhai::plugin::Dynamic| {
                    env.lock().println( &to_string( a0 ) );
                }
            );
        }

        {
            let env = env.clone();
            engine.register_result_fn(
                "println",
                move |a0: rhai::plugin::Dynamic, a1: rhai::plugin::Dynamic| {
                    let a0 = to_string( a0 );
                    let a1 = to_string( a1 );
                    let message = format( &a0, &[&a1] )?;
                    env.lock().println( &message );
                    Ok(())
                }
            );
        }

        {
            let env = env.clone();
            engine.register_result_fn(
                "println",
                move |a0: rhai::plugin::Dynamic, a1: rhai::plugin::Dynamic, a2: rhai::plugin::Dynamic| {
                    let a0 = to_string( a0 );
                    let a1 = to_string( a1 );
                    let a2 = to_string( a2 );
                    let message = format( &a0, &[&a1, &a2] )?;
                    env.lock().println( &message );
                    Ok(())
                }
            );
        }

        {
            let env = env.clone();
            engine.register_result_fn(
                "println",
                move |a0: rhai::plugin::Dynamic, a1: rhai::plugin::Dynamic, a2: rhai::plugin::Dynamic, a3: rhai::plugin::Dynamic| {
                    let a0 = to_string( a0 );
                    let a1 = to_string( a1 );
                    let a2 = to_string( a2 );
                    let a3 = to_string( a3 );
                    let message = format( &a0, &[&a1, &a2, &a3] )?;
                    env.lock().println( &message );
                    Ok(())
                }
            );
        }

        Engine {
            inner: engine
        }
    }

    pub fn run( &self, code: &str ) -> Result< Option< AllocationList >, EvalError > {
        match self.inner.eval::< rhai::plugin::Dynamic >( code ) {
            Ok( value ) => {
                if value.is::< AllocationList >() {
                    Ok( Some( value.cast::< AllocationList >() ) )
                } else {
                    Ok( None )
                }
            },
            Err( error ) => {
                let p = error.position();
                Err( EvalError {
                    message: error.to_string(),
                    line: p.line(),
                    column: p.position()
                })
            }
        }
    }
}

#[derive(Debug)]
pub struct EvalError {
    pub message: String,
    pub line: Option< usize >,
    pub column: Option< usize >
}

pub fn run_script( path: &Path, data_path: Option< &Path >, argv: Vec< String > ) -> Result< (), std::io::Error > {
    let mut args = EngineArgs {
        argv,
        .. EngineArgs::default()
    };

    if let Some( data_path ) = data_path {
        info!( "Loading {:?}...", data_path );
        let fp = File::open( &data_path )?;

        let debug_symbols: &[PathBuf] = &[];
        let data = Loader::load_from_stream( fp, debug_symbols )?;
        args.data = Some( Arc::new( data ) );
    }

    let env = Arc::new( Mutex::new( NativeEnvironment::default() ) );
    let engine = Engine::new( env, args );

    info!( "Running {:?}...", path );
    let result = engine.inner.eval_file::< rhai::plugin::Dynamic >( path.into() );
    match result {
        Ok( _ ) => {},
        Err( error ) => {
            error!( "{}", error );
            return Err( std::io::Error::new( std::io::ErrorKind::Other, "Failed to evaluate the script" ) );
        }
    }

    Ok(())
}

pub fn run_script_slave( data_path: Option< &Path > ) -> Result< (), std::io::Error > {
    let mut args = EngineArgs::default();

    if let Some( data_path ) = data_path {
        info!( "Loading {:?}...", data_path );
        let fp = File::open( &data_path )?;

        let debug_symbols: &[PathBuf] = &[];
        let data = Loader::load_from_stream( fp, debug_symbols )?;
        args.data = Some( Arc::new( data ) );
    }

    let env = Arc::new( Mutex::new( VirtualEnvironment::new() ) );
    let engine = Engine::new( env.clone(), args );
    let mut scope = rhai::Scope::new();
    let mut global_ast: rhai::AST = Default::default();

    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buffer = Vec::new();
    loop {
        use std::io::BufRead;

        buffer.clear();
        match stdin.read_until( 0, &mut buffer ) {
            Ok( count ) => {
                if count == 0 {
                    return Ok(());
                }
            },
            Err( _ ) => return Ok(())
        }

        if buffer.ends_with( b"\0" ) {
            buffer.pop();
        }

        let input = match std::str::from_utf8( &buffer ) {
            Ok( input ) => input,
            Err( _ ) => {
                let payload = serde_json::json! {{
                    "kind": "syntax_error",
                    "message": "invalid utf-8"
                }};

                println!( "{}", serde_json::to_string( &payload ).unwrap() );

                let payload = serde_json::json! {{
                    "kind": "idle"
                }};

                println!( "{}", serde_json::to_string( &payload ).unwrap() );

                continue;
            }
        };

        match engine.inner.compile_with_scope( &scope, &input ) {
            Ok( ast ) => {
                global_ast += ast;
                let result = engine.inner.eval_ast_with_scope::< rhai::Dynamic >( &mut scope, &global_ast );
                global_ast.clear_statements();

                let output = std::mem::take( &mut env.lock().output );
                for entry in output {
                    match entry {
                        ScriptOutputKind::PrintLine( message ) => {
                            let payload = serde_json::json! {{
                                "kind": "println",
                                "message": message,
                            }};

                            println!( "{}", serde_json::to_string( &payload ).unwrap() );
                        },
                        ScriptOutputKind::Image { path, data } => {
                            let payload = serde_json::json! {{
                                "kind": "image",
                                "path": path,
                                "data": &data[..]
                            }};

                            println!( "{}", serde_json::to_string( &payload ).unwrap() );
                        }
                    }
                }

                if let Err( error ) = result {
                    let p = error.position();
                    let payload = serde_json::json! {{
                        "kind": "runtime_error",
                        "message": error.to_string(),
                        "line": p.line(),
                        "column": p.position()
                    }};

                    println!( "{}", serde_json::to_string( &payload ).unwrap() );
                }
            },
            Err( error ) => {
                let p = error.1;
                let payload = serde_json::json! {{
                    "kind": "syntax_error",
                    "message": error.to_string(),
                    "line": p.line(),
                    "column": p.position()
                }};

                println!( "{}", serde_json::to_string( &payload ).unwrap() );
            }
        }

        let payload = serde_json::json! {{
            "kind": "idle"
        }};

        println!( "{}", serde_json::to_string( &payload ).unwrap() );
    }
}

struct ToCodeContext {
    allocation_source: String,
    output: String
}

impl Filter {
    pub fn to_code( &self, allocation_source: Option< String > ) -> String {
        let mut ctx = ToCodeContext {
            allocation_source: allocation_source.unwrap_or_else( || "allocations()".into() ),
            output: String::new()
        };

        self.to_code_impl( &mut ctx );

        ctx.output
    }
}

trait ToCode {
    fn to_code_impl( &self, ctx: &mut ToCodeContext );
}

impl ToCode for Filter {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        match *self {
            Filter::Basic( ref filter ) => filter.to_code_impl( ctx ),
            Filter::And( ref lhs, ref rhs ) => {
                write!( &mut ctx.output, "(" ).unwrap();
                lhs.to_code_impl( ctx );
                write!( &mut ctx.output, " & " ).unwrap();
                rhs.to_code_impl( ctx );
                write!( &mut ctx.output, ")" ).unwrap();
            },
            Filter::Or( ref lhs, ref rhs ) => {
                write!( &mut ctx.output, "(" ).unwrap();
                lhs.to_code_impl( ctx );
                write!( &mut ctx.output, " | " ).unwrap();
                rhs.to_code_impl( ctx );
                write!( &mut ctx.output, ")" ).unwrap();
            },
            Filter::Not( ref filter ) => {
                write!( &mut ctx.output, "(!" ).unwrap();
                filter.to_code_impl( ctx );
                write!( &mut ctx.output, ")" ).unwrap();
            }
        }
    }
}

impl ToCode for Regex {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        // TODO: Escape the string.
        write!( &mut ctx.output, "\"{}\"", self.as_str() ).unwrap();
    }
}

impl ToCode for u32 {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        write!( &mut ctx.output, "{}", self ).unwrap();
    }
}

impl ToCode for u64 {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        write!( &mut ctx.output, "{}", self ).unwrap();
    }
}

impl ToCode for usize {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        write!( &mut ctx.output, "{}", self ).unwrap();
    }
}

impl ToCode for NumberOrFractionOfTotal {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        match *self {
            NumberOrFractionOfTotal::Number( value ) => {
                write!( &mut ctx.output, "{}", value ).unwrap();
            },
            NumberOrFractionOfTotal::Fraction( value ) => {
                write!( &mut ctx.output, "{}", value ).unwrap();
            }
        }
    }
}

impl ToCode for Duration {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        if self.0.as_usecs() == 0 {
            write!( &mut ctx.output, "s(0)" ).unwrap();
            return;
        }

        let mut d = self.decompose();
        d.hours += d.days * 24;

        let mut non_empty = false;
        if d.hours > 0 {
            non_empty = true;
            write!( &mut ctx.output, "h({})", d.hours ).unwrap();
        }
        if d.minutes > 0 {
            if non_empty {
                ctx.output.push_str( " + " );
            }
            non_empty = true;
            write!( &mut ctx.output, "m({})", d.minutes ).unwrap();
        }
        if d.secs > 0 {
            if non_empty {
                ctx.output.push_str( " + " );
            }
            non_empty = true;
            write!( &mut ctx.output, "s({})", d.secs ).unwrap();
        }
        if d.ms > 0 {
            if non_empty {
                ctx.output.push_str( " + " );
            }
            non_empty = true;
            write!( &mut ctx.output, "ms({})", d.ms ).unwrap();
        }
        if d.us > 0 {
            if non_empty {
                ctx.output.push_str( " + " );
            }
            write!( &mut ctx.output, "us({})", d.us ).unwrap();
        }
    }
}

impl ToCode for HashSet< BacktraceId > {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        ctx.output.push_str( "[" );
        let mut is_first = true;
        for item in self {
            if is_first {
                is_first = false;
            } else {
                ctx.output.push_str( ", " );
            }
            write!( &mut ctx.output, "{}", item.raw() ).unwrap();
        }
        ctx.output.push_str( "]" );
    }
}

impl ToCode for BasicFilter {
    fn to_code_impl( &self, ctx: &mut ToCodeContext ) {
        macro_rules! out {
            ($($name:ident)+) => {
                $(
                    if let Some( ref value ) = self.$name {
                        write!( &mut ctx.output, "  .{}(", stringify!( $name ) ).unwrap();
                        value.to_code_impl( ctx );
                        writeln!( &mut ctx.output, ")" ).unwrap();
                    }
                )+
            }
        }

        macro_rules! out_bool {
            ($($name:ident)+) => {
                $(
                    if self.$name {
                        writeln!( &mut ctx.output, "  .{}()", stringify!( $name ) ).unwrap();
                    }
                )+
            }
        }

        writeln!( &mut ctx.output, "{}", ctx.allocation_source ).unwrap();

        out! {
            only_passing_through_function
            only_not_passing_through_function
            only_passing_through_source
            only_not_passing_through_source
            only_matching_backtraces
            only_backtrace_length_at_least
            only_backtrace_length_at_most
            only_larger_or_equal
            only_larger
            only_smaller_or_equal
            only_smaller
            only_address_at_least
            only_address_at_most
            only_allocated_after_at_least
            only_allocated_until_at_most
            only_deallocated_after_at_least
            only_deallocated_until_at_most
            only_not_deallocated_after_at_least
            only_not_deallocated_until_at_most
            only_alive_for_at_least
            only_alive_for_at_most
            only_leaked_or_deallocated_after
            only_first_size_larger_or_equal
            only_first_size_larger
            only_first_size_smaller_or_equal
            only_first_size_smaller
            only_last_size_larger_or_equal
            only_last_size_larger
            only_last_size_smaller_or_equal
            only_last_size_smaller
            only_chain_length_at_least
            only_chain_length_at_most
            only_chain_alive_for_at_least
            only_chain_alive_for_at_most

            only_group_allocations_at_least
            only_group_allocations_at_most
            only_group_interval_at_least
            only_group_interval_at_most
            only_group_leaked_allocations_at_least
            only_group_leaked_allocations_at_most

            only_with_marker
        }

        out_bool! {
            only_leaked
            only_temporary
            only_ptmalloc_mmaped
            only_ptmalloc_not_mmaped
            only_ptmalloc_from_main_arena
            only_ptmalloc_not_from_main_arena
        }
    }
}
