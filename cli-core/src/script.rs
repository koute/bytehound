use crate::data::OperationId;
use crate::exporter_flamegraph_pl::dump_collation_from_iter;
use crate::filter::{
    AllocationFilter, Compile, Duration, Filter, MapFilter, NumberOrFractionOfTotal,
    RawAllocationFilter, RawMapFilter, TryMatch,
};
use crate::timeline::{build_allocation_timeline, build_map_timeline};
use crate::{AllocationId, BacktraceId, Data, Loader, MapId, Timestamp, UsageDelta};
use ahash::AHashMap as HashMap;
use ahash::AHashSet as HashSet;
use parking_lot::Mutex;
use rayon::prelude::*;
use regex::Regex;
use std::cell::Cell;
use std::fmt::Write;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

pub use crate::script_virtual::ScriptOutputKind;
pub use crate::script_virtual::VirtualEnvironment;
pub use rhai;

struct DecomposedDuration {
    days: u64,
    hours: u64,
    minutes: u64,
    secs: u64,
    ms: u64,
    us: u64,
}

impl std::fmt::Display for DecomposedDuration {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut non_empty = false;
        if self.days > 0 {
            non_empty = true;
            write!(fmt, "{}d", self.days).unwrap();
        }
        if self.hours > 0 {
            if non_empty {
                fmt.write_str(" ").unwrap();
            }
            non_empty = true;
            write!(fmt, "{}h", self.hours).unwrap();
        }
        if self.minutes > 0 {
            if non_empty {
                fmt.write_str(" ").unwrap();
            }
            non_empty = true;
            write!(fmt, "{}m", self.minutes).unwrap();
        }
        if self.secs > 0 {
            if non_empty {
                fmt.write_str(" ").unwrap();
            }
            non_empty = true;
            write!(fmt, "{}s", self.secs).unwrap();
        }
        if self.ms > 0 {
            if non_empty {
                fmt.write_str(" ").unwrap();
            }
            non_empty = true;
            write!(fmt, "{}ms", self.ms).unwrap();
        }
        if self.us > 0 {
            if non_empty {
                fmt.write_str(" ").unwrap();
            }
            write!(fmt, "{}us", self.us).unwrap();
        }

        Ok(())
    }
}

impl Duration {
    fn decompose(self) -> DecomposedDuration {
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
            us,
        }
    }
}

fn dirname(path: &str) -> String {
    match std::path::Path::new(path).parent() {
        Some(parent) => parent.to_str().unwrap().into(),
        None => ".".into(),
    }
}

#[derive(Clone)]
struct DataRef(Arc<Data>);

impl std::fmt::Debug for DataRef {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "Data")
    }
}

impl std::ops::Deref for DataRef {
    type Target = Data;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DataRef {
    fn allocations(&mut self) -> AllocationList {
        AllocationList {
            data: self.clone(),
            allocation_ids: None,
            filter: None,
        }
    }

    fn maps(&mut self) -> MapList {
        MapList {
            data: self.clone(),
            map_ids: None,
            filter: None,
        }
    }
}

lazy_static::lazy_static! {
    static ref NOISY_FRAMES: HashSet< &'static str > = {
        let mut set = HashSet::new();
        let list = &[
            "core::ops::function::FnOnce::call_once",
            "core::ops::function::impls::<impl core::ops::function::FnOnce<A> for &F>::call_once",
            "std::panic::catch_unwind",
            "std::panicking::try::do_call",
            "std::panicking::try",
            "std::rt::lang_start_internal::{{closure}}",
            "std::rt::lang_start_internal",
            "std::rt::lang_start::{{closure}}",
            "std::rt::lang_start",
            "std::sys_common::backtrace::__rust_begin_short_backtrace",
        ];
        for &entry in list {
            set.insert( entry );
        }
        set
    };
    static ref TERMINAL_FRAMES: HashSet< &'static str > = {
        let mut set = HashSet::new();
        let list = &[
            "alloc::vec::Vec<T,A>::resize",
        ];
        for &entry in list {
            set.insert( entry );
        }
        set
    };
}

#[derive(Clone)]
pub struct Backtrace {
    data: DataRef,
    id: BacktraceId,
    strip: bool,
}

impl Backtrace {
    fn write_to(&self, mut fmt: impl std::fmt::Write) -> std::fmt::Result {
        let mut is_first = true;
        let interner = self.data.interner();
        for (index, (_, frame)) in self.data.get_backtrace(self.id).enumerate() {
            let function = frame
                .any_function()
                .map(|function| interner.resolve(function).unwrap());
            if self.strip {
                if let Some(function) = function {
                    if NOISY_FRAMES.contains(function) {
                        continue;
                    }
                }
            }
            if !is_first {
                write!(fmt, "\n")?;
            }

            is_first = false;

            write!(fmt, "#{:02}", index)?;
            if let Some(library) = frame.library() {
                write!(fmt, " [{}]", interner.resolve(library).unwrap())?;
            }
            if let Some(function) = function {
                write!(fmt, " {}", function)?;
            } else {
                write!(fmt, " {:0x}", frame.address().raw())?;
            }
            if let Some(source) = frame.source() {
                let mut source = interner.resolve(source).unwrap();
                if let Some(index) = source.rfind("/") {
                    source = &source[index + 1..];
                }
                write!(fmt, " [{}", source)?;
                if let Some(line) = frame.line() {
                    write!(fmt, ":{}", line)?;
                }
                write!(fmt, "]")?;
            }

            if self.strip {
                if let Some(function) = function {
                    if TERMINAL_FRAMES.contains(function) {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

impl std::fmt::Debug for Backtrace {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "Backtrace")
    }
}

impl std::fmt::Display for Backtrace {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.write_to(fmt)
    }
}

#[derive(Clone)]
pub struct Allocation {
    data: DataRef,
    id: AllocationId,
}

impl std::fmt::Debug for Allocation {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "Allocation")
    }
}

#[derive(Clone)]
pub struct AllocationList {
    data: DataRef,
    allocation_ids: Option<Arc<Vec<AllocationId>>>,
    filter: Option<AllocationFilter>,
}

impl std::fmt::Debug for AllocationList {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "AllocationList")
    }
}

#[derive(Clone)]
pub struct Map {
    data: DataRef,
    id: MapId,
}

impl std::fmt::Debug for Map {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "Map")
    }
}

#[derive(Clone)]
pub struct MapList {
    data: DataRef,
    map_ids: Option<Arc<Vec<MapId>>>,
    filter: Option<MapFilter>,
}

impl std::fmt::Debug for MapList {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "MapList")
    }
}

// This was copied from the `plotters` crate.
fn gen_keypoints(range: (u64, u64), max_points: usize) -> Vec<u64> {
    let mut scale: u64 = 1;
    let range = (range.0.min(range.1), range.0.max(range.1));
    'outer: while (range.1 - range.0 + scale - 1) as usize / (scale as usize) > max_points {
        let next_scale = scale * 10;
        for new_scale in [scale * 2, scale * 5, scale * 10].iter() {
            scale = *new_scale;
            if (range.1 - range.0 + *new_scale - 1) as usize / (*new_scale as usize) < max_points {
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

fn to_chrono(timestamp: u64) -> chrono::DateTime<chrono::Utc> {
    use chrono::prelude::*;

    let secs = timestamp / 1_000_000;
    Utc.timestamp_opt(secs as i64, ((timestamp - secs * 1_000_000) * 1000) as u32)
        .unwrap()
}

fn expand_datapoints<V>(xs: &[u64], datapoints: &[(u64, V)]) -> Vec<(u64, V)>
where
    V: Copy + Default,
{
    if xs.is_empty() {
        return Vec::new();
    }

    if datapoints.is_empty() {
        return xs.iter().map(|&x| (x, Default::default())).collect();
    }

    assert!(xs.len() >= datapoints.len());
    assert!(xs[0] <= datapoints[0].0);
    assert!(xs[xs.len() - 1] >= datapoints[datapoints.len() - 1].0);

    let mut expanded = Vec::with_capacity(xs.len());
    let mut last_value = Default::default();
    let mut dense = xs.iter().copied();
    let mut sparse = datapoints.iter().copied();

    while let Some(mut dense_key) = dense.next() {
        if let Some((sparse_key, value)) = sparse.next() {
            if dense_key < sparse_key {
                while dense_key < sparse_key {
                    expanded.push((dense_key, last_value));
                    dense_key = dense.next().unwrap();
                }
            } else if dense_key > sparse_key {
                unreachable!();
            }

            expanded.push((dense_key, value));
            last_value = value;
        } else {
            expanded.push((dense_key, last_value));
        }
    }

    assert_eq!(xs.len(), expanded.len());
    expanded
}

#[test]
fn test_expand_datapoints() {
    assert_eq!(
        expand_datapoints(&[0, 1, 2], &[(1, 100)]),
        &[(0, 0), (1, 100), (2, 100)]
    );

    assert_eq!(
        expand_datapoints(&[0, 1, 2, 3], &[(0, 100), (2, 200)]),
        &[(0, 100), (1, 100), (2, 200), (3, 200)]
    );
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum OpFilter {
    Both,
    OnlyAlloc,
    None,
}

fn get_timestamp(data: &Data, op: OperationId) -> common::Timestamp {
    if op.is_allocation() || op.is_reallocation() {
        data.get_allocation(op.id()).timestamp
    } else {
        data.get_allocation(op.id())
            .deallocation
            .as_ref()
            .unwrap()
            .timestamp
    }
}

fn filtered_ids<'a, T>(list: &'a T) -> impl ParallelIterator<Item = <T as List>::Id> + 'a
where
    T: List + Send + Sync,
{
    let filter = list
        .filter_ref()
        .map(|filter| filter.compile(list.data_ref()));
    list.unfiltered_ids_par_iter().filter(move |&id| {
        let allocation = list.get_native_item(id);
        if let Some(ref filter) = filter {
            filter.try_match(list.data_ref(), allocation)
        } else {
            true
        }
    })
}

trait List: Sized + Send + Sync {
    type Id: Copy + Send + Sync + PartialEq + Eq + std::hash::Hash;
    type RawFilter: Clone + Default + Compile;
    type Item;

    fn create(
        data: DataRef,
        unfiltered_ids: Option<Arc<Vec<Self::Id>>>,
        filter: Option<Filter<Self::RawFilter>>,
    ) -> Self;
    fn create_item(&self, id: Self::Id) -> Self::Item;
    fn data_ref(&self) -> &DataRef;
    fn filter_ref(&self) -> Option<&Filter<Self::RawFilter>>;
    fn unfiltered_ids_ref(&self) -> Option<&Arc<Vec<Self::Id>>>;
    fn get_native_item(
        &self,
        id: Self::Id,
    ) -> &<<Self::RawFilter as Compile>::Compiled as TryMatch>::Item;
    fn default_unfiltered_ids(data: &Data) -> &[Self::Id];
    fn list_by_backtrace(data: &Data, backtrace: BacktraceId) -> Vec<Self::Id>;

    fn unfiltered_ids(&self) -> &[Self::Id] {
        self.unfiltered_ids_ref()
            .map(|map_ids| map_ids.as_slice())
            .unwrap_or_else(|| Self::default_unfiltered_ids(&self.data_ref()))
    }

    fn unfiltered_ids_iter(&self) -> std::iter::Copied<std::slice::Iter<Self::Id>> {
        self.unfiltered_ids().iter().copied()
    }

    fn unfiltered_ids_par_iter(&self) -> rayon::iter::Copied<rayon::slice::Iter<Self::Id>> {
        self.unfiltered_ids().par_iter().copied()
    }

    fn len(&mut self) -> i64 {
        self.apply_filter();
        self.unfiltered_ids().len() as i64
    }

    fn apply_filter(&mut self) {
        if self.filter_ref().is_none() {
            return;
        }

        let list: Vec<_> = filtered_ids(self).collect();
        *self = Self::create(self.data_ref().clone(), Some(Arc::new(list)), None);
    }

    fn clone_with_filter(&self, filter: Option<Filter<Self::RawFilter>>) -> Self {
        Self::create(
            self.data_ref().clone(),
            self.unfiltered_ids_ref().cloned(),
            filter,
        )
    }

    fn uses_same_list(&self, rhs: &Self) -> bool {
        match (self.unfiltered_ids_ref(), rhs.unfiltered_ids_ref()) {
            (None, None) => true,
            (Some(lhs), Some(rhs)) => Arc::ptr_eq(lhs, rhs),
            _ => false,
        }
    }

    fn add_filter(&self, callback: impl FnOnce(&mut Self::RawFilter)) -> Self {
        self.add_filter_once(|_| false, callback)
    }

    fn add_filter_once(
        &self,
        is_filled: impl FnOnce(&Self::RawFilter) -> bool,
        callback: impl FnOnce(&mut Self::RawFilter),
    ) -> Self {
        let filter = match self.filter_ref() {
            None => {
                let mut new_filter = Self::RawFilter::default();
                callback(&mut new_filter);

                Filter::Basic(new_filter)
            }
            Some(Filter::Basic(ref old_filter)) => {
                if is_filled(old_filter) {
                    let mut new_filter = Self::RawFilter::default();
                    callback(&mut new_filter);

                    Filter::And(
                        Box::new(Filter::Basic(old_filter.clone())),
                        Box::new(Filter::Basic(new_filter)),
                    )
                } else {
                    let mut new_filter = old_filter.clone();
                    callback(&mut new_filter);

                    Filter::Basic(new_filter)
                }
            }
            Some(Filter::And(ref lhs, ref rhs)) if matches!(**rhs, Filter::Basic(_)) => match **rhs
            {
                Filter::Basic(ref old_filter) => {
                    let mut new_filter = old_filter.clone();
                    callback(&mut new_filter);

                    Filter::And(lhs.clone(), Box::new(Filter::Basic(new_filter)))
                }
                _ => unreachable!(),
            },
            Some(old_filter) => {
                let mut new_filter = Self::RawFilter::default();
                callback(&mut new_filter);

                Filter::And(
                    Box::new(old_filter.clone()),
                    Box::new(Filter::Basic(new_filter)),
                )
            }
        };

        self.clone_with_filter(Some(filter))
    }

    fn rhai_merge(mut lhs: Self, mut rhs: Self) -> Result<Self, Box<rhai::EvalAltResult>> {
        if lhs.data_ref().id != rhs.data_ref().id {
            return Err(Box::new(rhai::EvalAltResult::from(
                "lists don't come from the same data file",
            )));
        }

        if lhs.uses_same_list(&rhs) {
            let filter = match (lhs.filter_ref(), rhs.filter_ref()) {
                (Some(lhs), Some(rhs)) => {
                    Some(Filter::Or(Box::new(lhs.clone()), Box::new(rhs.clone())))
                }
                _ => None,
            };

            Ok(lhs.clone_with_filter(filter))
        } else {
            lhs.apply_filter();
            rhs.apply_filter();

            let mut set: HashSet<Self::Id> = HashSet::new();
            set.extend(lhs.unfiltered_ids_iter());
            set.extend(rhs.unfiltered_ids_iter());

            let ids: Vec<_> = Self::create(lhs.data_ref().clone(), None, None)
                .unfiltered_ids_par_iter()
                .filter(|id| set.contains(&id))
                .collect();
            Ok(Self::create(
                lhs.data_ref().clone(),
                Some(Arc::new(ids)),
                None,
            ))
        }
    }

    fn rhai_substract(lhs: Self, mut rhs: Self) -> Result<Self, Box<rhai::EvalAltResult>> {
        if lhs.data_ref().id != rhs.data_ref().id {
            return Err(Box::new(rhai::EvalAltResult::from(
                "lists don't come from the same data file",
            )));
        }

        if lhs.uses_same_list(&rhs) {
            let filter = match (lhs.filter_ref(), rhs.filter_ref()) {
                (_, None) => {
                    return Ok(Self::create(
                        lhs.data_ref().clone(),
                        Some(Arc::new(Vec::new())),
                        None,
                    ));
                }
                (None, Some(rhs)) => Some(Filter::Not(Box::new(rhs.clone()))),
                (Some(lhs), Some(rhs)) => Some(Filter::And(
                    Box::new(lhs.clone()),
                    Box::new(Filter::Not(Box::new(rhs.clone()))),
                )),
            };

            Ok(lhs.clone_with_filter(filter))
        } else {
            rhs.apply_filter();

            let mut set: HashSet<Self::Id> = HashSet::new();
            set.extend(rhs.unfiltered_ids_iter());

            let ids: Vec<_> = filtered_ids(&lhs).filter(|id| !set.contains(id)).collect();
            Ok(Self::create(
                lhs.data_ref().clone(),
                Some(Arc::new(ids)),
                None,
            ))
        }
    }

    fn rhai_intersect(lhs: Self, mut rhs: Self) -> Result<Self, Box<rhai::EvalAltResult>> {
        if lhs.data_ref().id != rhs.data_ref().id {
            return Err(Box::new(rhai::EvalAltResult::from(
                "lists don't come from the same data file",
            )));
        }

        if lhs.uses_same_list(&rhs) {
            let filter = match (lhs.filter_ref(), rhs.filter_ref()) {
                (None, None) => None,
                (Some(lhs), None) => Some(lhs.clone()),
                (None, Some(rhs)) => Some(rhs.clone()),
                (Some(lhs), Some(rhs)) => {
                    Some(Filter::And(Box::new(lhs.clone()), Box::new(rhs.clone())))
                }
            };

            Ok(lhs.clone_with_filter(filter))
        } else {
            rhs.apply_filter();

            let mut set: HashSet<Self::Id> = HashSet::new();
            set.extend(rhs.unfiltered_ids_iter());

            let ids: Vec<_> = filtered_ids(&lhs).filter(|id| set.contains(id)).collect();
            Ok(Self::create(
                lhs.data_ref().clone(),
                Some(Arc::new(ids)),
                None,
            ))
        }
    }

    fn rhai_get(&mut self, index: i64) -> Result<Self::Item, Box<rhai::EvalAltResult>> {
        self.apply_filter();
        let list = self.unfiltered_ids();
        let id = list
            .get(index as usize)
            .ok_or_else(|| error("index out of range"))?;
        Ok(self.create_item(*id))
    }
}

impl List for MapList {
    type Id = MapId;
    type RawFilter = RawMapFilter;
    type Item = Map;

    fn create(
        data: DataRef,
        unfiltered_ids: Option<Arc<Vec<Self::Id>>>,
        filter: Option<Filter<Self::RawFilter>>,
    ) -> Self {
        MapList {
            data,
            map_ids: unfiltered_ids,
            filter,
        }
    }

    fn create_item(&self, id: Self::Id) -> Self::Item {
        Map {
            data: self.data.clone(),
            id,
        }
    }

    fn data_ref(&self) -> &DataRef {
        &self.data
    }

    fn filter_ref(&self) -> Option<&Filter<Self::RawFilter>> {
        self.filter.as_ref()
    }

    fn unfiltered_ids_ref(&self) -> Option<&Arc<Vec<Self::Id>>> {
        self.map_ids.as_ref()
    }

    fn get_native_item(
        &self,
        id: Self::Id,
    ) -> &<<Self::RawFilter as Compile>::Compiled as TryMatch>::Item {
        &self.data.0.maps()[id.0 as usize]
    }

    fn default_unfiltered_ids(data: &Data) -> &[Self::Id] {
        &data.map_ids
    }

    fn list_by_backtrace(data: &Data, backtrace: BacktraceId) -> Vec<Self::Id> {
        // TODO: Cache this.
        data.maps()
            .iter()
            .enumerate()
            .filter(|(_, map)| {
                map.source
                    .map(|source| source.backtrace == backtrace)
                    .unwrap_or(false)
            })
            .map(|(index, _)| MapId(index as u64))
            .collect()
    }
}

impl List for AllocationList {
    type Id = AllocationId;
    type RawFilter = RawAllocationFilter;
    type Item = Allocation;

    fn create(
        data: DataRef,
        unfiltered_ids: Option<Arc<Vec<Self::Id>>>,
        filter: Option<Filter<Self::RawFilter>>,
    ) -> Self {
        AllocationList {
            data,
            allocation_ids: unfiltered_ids,
            filter,
        }
    }

    fn create_item(&self, id: Self::Id) -> Self::Item {
        Allocation {
            data: self.data.clone(),
            id,
        }
    }

    fn data_ref(&self) -> &DataRef {
        &self.data
    }

    fn filter_ref(&self) -> Option<&Filter<Self::RawFilter>> {
        self.filter.as_ref()
    }

    fn unfiltered_ids_ref(&self) -> Option<&Arc<Vec<Self::Id>>> {
        self.allocation_ids.as_ref()
    }

    fn get_native_item(
        &self,
        id: Self::Id,
    ) -> &<<Self::RawFilter as Compile>::Compiled as TryMatch>::Item {
        self.data.get_allocation(id)
    }

    fn default_unfiltered_ids(data: &Data) -> &[Self::Id] {
        &data.sorted_by_timestamp
    }

    fn list_by_backtrace(data: &Data, id: BacktraceId) -> Vec<Self::Id> {
        data.get_allocation_ids_by_backtrace(id).to_owned()
    }
}

impl AllocationList {
    pub fn allocation_ids(&mut self) -> &[AllocationId] {
        self.apply_filter();
        self.unfiltered_ids()
    }

    fn save_as_flamegraph_to_string(&mut self) -> Result<String, Box<rhai::EvalAltResult>> {
        self.apply_filter();

        let mut lines = Vec::new();
        let iter = self
            .unfiltered_ids_iter()
            .map(|allocation_id| (allocation_id, self.data.get_allocation(allocation_id)));

        dump_collation_from_iter(&self.data, iter, |line| {
            lines.push(line.to_owned());
            let result: Result<(), ()> = Ok(());
            result
        })
        .map_err(|_| Box::new(rhai::EvalAltResult::from("failed to collate allocations")))?;

        lines.sort_unstable();

        let mut output = String::new();
        crate::exporter_flamegraph::lines_to_svg(lines, &mut output);

        Ok(output)
    }

    fn save_as_flamegraph(
        &mut self,
        env: &mut dyn Environment,
        path: String,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        let data = self.save_as_flamegraph_to_string()?;
        env.file_write(&path, FileKind::Svg, data.as_bytes())?;
        Ok(self.clone())
    }

    fn save_as_graph(
        &self,
        env: &mut dyn Environment,
        path: String,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        Graph::new().add(self.clone())?.save(env, path)?;
        Ok(self.clone())
    }

    fn filtered_ops(
        &mut self,
        mut callback: impl FnMut(AllocationId) -> OpFilter,
    ) -> Vec<OperationId> {
        self.apply_filter();
        let ids = self.unfiltered_ids_iter();
        let mut ops = Vec::with_capacity(ids.len());
        for id in ids {
            let filter = callback(id);
            if filter == OpFilter::None {
                continue;
            }

            let allocation = self.data.get_allocation(id);
            ops.push(OperationId::new_allocation(id));

            if allocation.deallocation.is_some() && filter != OpFilter::OnlyAlloc {
                ops.push(OperationId::new_deallocation(id));
            }
        }

        ops.par_sort_by_key(|op| get_timestamp(&self.data, *op));
        ops
    }

    fn group_by_backtrace(&mut self) -> AllocationGroupList {
        #[derive(Default)]
        struct Group {
            allocation_ids: Vec<AllocationId>,
            size: u64,
        }

        self.apply_filter();
        let mut groups = HashMap::new();
        for id in self.unfiltered_ids_iter() {
            let allocation = self.data.get_allocation(id);
            let group = groups
                .entry(allocation.backtrace)
                .or_insert_with(|| Group::default());
            group.size += allocation.size;
            group.allocation_ids.push(id);
        }

        AllocationGroupList {
            data: self.data.clone(),
            groups: Arc::new(
                groups
                    .into_iter()
                    .map(|(_, group)| AllocationGroupInner {
                        allocation_ids: Arc::new(group.allocation_ids),
                        size: group.size,
                    })
                    .collect(),
            ),
        }
    }
}

impl MapList {
    pub fn map_ids(&mut self) -> &[MapId] {
        self.apply_filter();
        self.unfiltered_ids()
    }
}

#[derive(Clone)]
struct AllocationGroupInner {
    allocation_ids: Arc<Vec<AllocationId>>,
    size: u64,
}

struct AllocationGroupListIter {
    group_list: AllocationGroupList,
    index: usize,
}

impl Iterator for AllocationGroupListIter {
    type Item = AllocationList;
    fn next(&mut self) -> Option<Self::Item> {
        let group = self.group_list.groups.get(self.index)?;
        self.index += 1;

        Some(AllocationList {
            data: self.group_list.data.clone(),
            allocation_ids: Some(group.allocation_ids.clone()),
            filter: None,
        })
    }
}

#[derive(Clone)]
struct AllocationGroupList {
    data: DataRef,
    groups: Arc<Vec<AllocationGroupInner>>,
}

impl std::fmt::Debug for AllocationGroupList {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "AllocationGroupList")
    }
}

impl IntoIterator for AllocationGroupList {
    type Item = AllocationList;
    type IntoIter = AllocationGroupListIter;

    fn into_iter(self) -> Self::IntoIter {
        AllocationGroupListIter {
            group_list: self,
            index: 0,
        }
    }
}

impl AllocationGroupList {
    fn filter(&self, callback: impl Fn(&AllocationGroupInner) -> bool + Send + Sync) -> Self {
        let groups: Vec<_> = self
            .groups
            .par_iter()
            .filter(|group| callback(group))
            .map(|group| group.clone())
            .collect();

        Self {
            data: self.data.clone(),
            groups: Arc::new(groups.into_iter().collect()),
        }
    }

    fn sort_by_key<T>(&self, callback: impl Fn(&AllocationGroupInner) -> T + Send + Sync) -> Self
    where
        T: Ord,
    {
        let mut groups = (*self.groups).clone();
        groups.par_sort_by_key(callback);

        AllocationGroupList {
            data: self.data.clone(),
            groups: Arc::new(groups),
        }
    }

    fn len(&mut self) -> i64 {
        self.groups.len() as i64
    }

    fn only_all_leaked(&mut self) -> AllocationGroupList {
        self.filter(|group| {
            group.allocation_ids.par_iter().all(|&id| {
                let allocation = self.data.get_allocation(id);
                allocation.deallocation.is_none()
            })
        })
    }

    fn only_count_at_least(&mut self, count: i64) -> AllocationGroupList {
        self.filter(|group| group.allocation_ids.len() as i64 >= count)
    }

    fn sort_by_size_ascending(&mut self) -> AllocationGroupList {
        self.sort_by_key(|group| group.size)
    }

    fn sort_by_size_descending(&mut self) -> AllocationGroupList {
        self.sort_by_key(|group| !group.size)
    }

    fn sort_by_count_ascending(&mut self) -> AllocationGroupList {
        self.sort_by_key(|group| group.allocation_ids.len())
    }

    fn sort_by_count_descending(&mut self) -> AllocationGroupList {
        self.sort_by_key(|group| !group.allocation_ids.len())
    }

    fn ungroup(&mut self) -> AllocationList {
        let mut allocation_ids = Vec::new();
        for group in &*self.groups {
            allocation_ids.extend_from_slice(&group.allocation_ids);
        }
        allocation_ids.par_sort_by_key(|&id| {
            let allocation = self.data.get_allocation(id);
            (allocation.timestamp, id)
        });

        AllocationList {
            data: self.data.clone(),
            allocation_ids: Some(Arc::new(allocation_ids)),
            filter: None,
        }
    }

    fn get(&mut self, index: i64) -> Result<AllocationList, Box<rhai::EvalAltResult>> {
        let group = self
            .groups
            .get(index as usize)
            .ok_or_else(|| error("index out of range"))?;
        Ok(AllocationList {
            data: self.data.clone(),
            allocation_ids: Some(group.allocation_ids.clone()),
            filter: None,
        })
    }

    fn take(&mut self, count: i64) -> Self {
        let length = std::cmp::min(self.groups.len(), count as usize);

        AllocationGroupList {
            data: self.data.clone(),
            groups: Arc::new(self.groups[..length].to_owned()),
        }
    }
}

#[derive(Copy, Clone)]
enum AllocationGraphKind {
    MemoryUsage,
    LiveAllocations,
    NewAllocations,
    Deallocations,
}

#[derive(Copy, Clone)]
enum MapGraphKind {
    RSS,
    AddressSpace,
}

#[derive(Copy, Clone)]
enum GraphKind {
    Allocation(AllocationGraphKind),
    Map(MapGraphKind),
}

impl GraphKind {
    fn is_for_allocations(&self) -> bool {
        matches!(self, GraphKind::Allocation(..))
    }

    fn is_for_maps(&self) -> bool {
        matches!(self, GraphKind::Map(..))
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
    start_at: Option<Duration>,
    end_at: Option<Duration>,
    allocation_lists: Vec<AllocationList>,
    map_lists: Vec<MapList>,
    labels: Vec<Option<String>>,
    gradient: Option<Arc<colorgrad::Gradient>>,
    kind: Option<GraphKind>,

    cached_datapoints: Option<Arc<(Vec<u64>, Vec<Vec<(u64, u64)>>)>>,
}

fn finalize_datapoints(
    mut xs: Vec<u64>,
    mut datapoints_for_ops: Vec<Vec<(u64, u64)>>,
) -> (Vec<u64>, Vec<Vec<(u64, u64)>>) {
    xs.sort_unstable();

    for datapoints in &mut datapoints_for_ops {
        if datapoints.is_empty() {
            continue;
        }
        *datapoints = expand_datapoints(&xs, &datapoints);
    }

    for index in 0..xs.len() {
        let mut value = 0;
        for datapoints in datapoints_for_ops.iter_mut() {
            if datapoints.is_empty() {
                continue;
            }

            value += datapoints[index].1;
            datapoints[index].1 = value;
        }
    }

    (xs, datapoints_for_ops)
}

fn prepare_allocation_graph_datapoints(
    data: &Data,
    ops_for_list: &[Vec<OperationId>],
    kind: AllocationGraphKind,
) -> (Vec<u64>, Vec<Vec<(u64, u64)>>) {
    let timestamp_min = ops_for_list
        .iter()
        .flat_map(|ops| ops.first())
        .map(|op| get_timestamp(&data, *op))
        .min()
        .unwrap_or(common::Timestamp::min());
    let timestamp_max = ops_for_list
        .iter()
        .flat_map(|ops| ops.last())
        .map(|op| get_timestamp(&data, *op))
        .max()
        .unwrap_or(common::Timestamp::min());

    let mut xs = HashSet::new();
    let mut datapoints_for_ops = Vec::new();
    for ops in ops_for_list {
        if ops.is_empty() {
            datapoints_for_ops.push(Vec::new());
            continue;
        }

        let datapoints: Vec<_> =
            build_allocation_timeline(&data, timestamp_min, timestamp_max, ops)
                .into_iter()
                .map(|point| {
                    xs.insert(point.timestamp);
                    let x = point.timestamp;
                    let y = match kind {
                        AllocationGraphKind::MemoryUsage => point.memory_usage,
                        AllocationGraphKind::LiveAllocations => point.allocations,
                        AllocationGraphKind::NewAllocations => point.positive_change.allocations,
                        AllocationGraphKind::Deallocations => point.negative_change.allocations,
                    } as u64;
                    (x, y)
                })
                .collect();

        datapoints_for_ops.push(datapoints);
    }

    finalize_datapoints(xs.into_iter().collect(), datapoints_for_ops)
}

fn prepare_map_graph_datapoints(
    ops_for_list: &[Vec<(Timestamp, UsageDelta)>],
    kind: MapGraphKind,
) -> (Vec<u64>, Vec<Vec<(u64, u64)>>) {
    let timestamp_min = ops_for_list
        .iter()
        .flat_map(|ops| ops.first())
        .map(|(timestamp, _)| *timestamp)
        .min()
        .unwrap_or(common::Timestamp::min());
    let timestamp_max = ops_for_list
        .iter()
        .flat_map(|ops| ops.last())
        .map(|(timestamp, _)| *timestamp)
        .max()
        .unwrap_or(common::Timestamp::min());

    let mut xs = HashSet::new();
    let mut datapoints_for_ops = Vec::new();
    for ops in ops_for_list {
        if ops.is_empty() {
            datapoints_for_ops.push(Vec::new());
            continue;
        }

        let datapoints: Vec<_> = build_map_timeline(timestamp_min, timestamp_max, ops)
            .into_iter()
            .map(|point| {
                xs.insert(point.timestamp);
                let x = point.timestamp;
                let y = match kind {
                    MapGraphKind::RSS => point.rss(),
                    MapGraphKind::AddressSpace => point.address_space,
                } as u64;
                (x, y)
            })
            .collect();

        datapoints_for_ops.push(datapoints);
    }

    finalize_datapoints(xs.into_iter().collect(), datapoints_for_ops)
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
            start_at: None,
            end_at: None,
            allocation_lists: Vec::new(),
            map_lists: Vec::new(),
            labels: Vec::new(),
            gradient: None,
            kind: None,

            cached_datapoints: None,
        }
    }

    fn add_with_label(
        &mut self,
        label: String,
        list: AllocationList,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let mut cloned = self.clone();
        cloned.allocation_lists.push(list);
        cloned.labels.push(Some(label));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn add_group(&mut self, group: AllocationGroupList) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let mut cloned = self.clone();
        for list in group {
            cloned.allocation_lists.push(list);
            cloned.labels.push(None);
        }
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn add(&mut self, list: AllocationList) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let mut cloned = self.clone();
        cloned.allocation_lists.push(list);
        cloned.labels.push(None);
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn add_maps_with_label(
        &mut self,
        label: String,
        list: MapList,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_map_graph()?;
        let mut cloned = self.clone();
        cloned.map_lists.push(list);
        cloned.labels.push(Some(label));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn add_maps(&mut self, list: MapList) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_map_graph()?;
        let mut cloned = self.clone();
        cloned.map_lists.push(list);
        cloned.labels.push(None);
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn only_non_empty_series(&mut self) -> Self {
        let mut cloned = self.clone();
        cloned.hide_empty = true;
        cloned
    }

    fn trim(&mut self) -> Self {
        let mut cloned = self.clone();
        cloned.trim_left = true;
        cloned.trim_right = true;
        cloned
    }

    fn trim_left(&mut self) -> Self {
        let mut cloned = self.clone();
        cloned.trim_left = true;
        cloned
    }

    fn trim_right(&mut self) -> Self {
        let mut cloned = self.clone();
        cloned.trim_right = true;
        cloned
    }

    fn start_at(&mut self, offset: Duration) -> Self {
        let mut cloned = self.clone();
        cloned.start_at = Some(offset);
        cloned
    }

    fn end_at(&mut self, offset: Duration) -> Self {
        let mut cloned = self.clone();
        cloned.end_at = Some(offset);
        cloned
    }

    fn without_legend(&mut self) -> Self {
        let mut cloned = self.clone();
        cloned.without_legend = true;
        cloned
    }

    fn without_axes(&mut self) -> Self {
        let mut cloned = self.clone();
        cloned.without_axes = true;
        cloned
    }

    fn without_grid(&mut self) -> Self {
        let mut cloned = self.clone();
        cloned.without_grid = true;
        cloned
    }

    fn bail_unless_allocation_graph(&self) -> Result<(), Box<rhai::EvalAltResult>> {
        if !self
            .graph_kind()
            .map(|kind| kind.is_for_allocations())
            .unwrap_or(true)
        {
            return Err(error("not an allocation graph"));
        }
        Ok(())
    }

    fn bail_unless_map_graph(&self) -> Result<(), Box<rhai::EvalAltResult>> {
        if !self
            .graph_kind()
            .map(|kind| kind.is_for_maps())
            .unwrap_or(true)
        {
            return Err(error("not a map graph"));
        }
        Ok(())
    }

    fn show_memory_usage(&mut self) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let mut cloned = self.clone();
        cloned.kind = Some(GraphKind::Allocation(AllocationGraphKind::MemoryUsage));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn show_live_allocations(&mut self) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let mut cloned = self.clone();
        cloned.kind = Some(GraphKind::Allocation(AllocationGraphKind::LiveAllocations));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn show_new_allocations(&mut self) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let mut cloned = self.clone();
        cloned.kind = Some(GraphKind::Allocation(AllocationGraphKind::NewAllocations));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn show_deallocations(&mut self) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let mut cloned = self.clone();
        cloned.kind = Some(GraphKind::Allocation(AllocationGraphKind::Deallocations));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn show_rss(&mut self) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_map_graph()?;
        let mut cloned = self.clone();
        cloned.kind = Some(GraphKind::Map(MapGraphKind::RSS));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn show_address_space(&mut self) -> Result<Self, Box<rhai::EvalAltResult>> {
        self.bail_unless_map_graph()?;
        let mut cloned = self.clone();
        cloned.kind = Some(GraphKind::Map(MapGraphKind::AddressSpace));
        cloned.cached_datapoints = None;
        Ok(cloned)
    }

    fn generate_allocation_ops(
        &mut self,
    ) -> Result<Vec<Vec<OperationId>>, Box<rhai::EvalAltResult>> {
        self.bail_unless_allocation_graph()?;
        let lists = &mut self.allocation_lists;
        if lists.is_empty() {
            return Err(error("no allocation lists given"));
        }

        let data = lists[0].data.clone();
        if !lists.iter().all(|list| list.data.id() == data.id()) {
            return Err(error(
                "not every allocation list given is from the same data file",
            ));
        }

        let threshold = self
            .end_at
            .map(|offset| data.initial_timestamp + offset.0)
            .unwrap_or(data.last_timestamp);

        let mut seen = HashSet::new();
        let ops_for_list: Vec<_> = lists
            .iter_mut()
            .map(|list| {
                list.filtered_ops(|id| {
                    if !seen.insert(id) {
                        return OpFilter::None;
                    }

                    let allocation = data.get_allocation(id);
                    if allocation.timestamp > threshold {
                        return OpFilter::None;
                    }

                    if let Some(ref deallocation) = allocation.deallocation {
                        if deallocation.timestamp > threshold {
                            return OpFilter::OnlyAlloc;
                        }
                    }

                    OpFilter::Both
                })
            })
            .collect();

        Ok(ops_for_list)
    }

    fn generate_map_ops(
        &mut self,
    ) -> Result<Vec<Vec<(Timestamp, UsageDelta)>>, Box<rhai::EvalAltResult>> {
        self.bail_unless_map_graph()?;
        let lists = &mut self.map_lists;
        if lists.is_empty() {
            return Err(error("no allocation lists given"));
        }

        let data = lists[0].data.clone();
        if !lists.iter().all(|list| list.data.id() == data.id()) {
            return Err(error("not every map list given is from the same data file"));
        }

        for list in lists.iter_mut() {
            list.apply_filter();
        }

        let mut seen = HashSet::new();
        let ops_for_list: Vec<_> = lists
            .iter()
            .map(|list| {
                let ids = list.unfiltered_ids_iter();
                let mut ops = Vec::with_capacity(ids.len());
                for map_id in ids {
                    if !seen.insert(map_id) {
                        continue;
                    }
                    data.get_map(map_id).emit_ops(&mut ops);
                }

                ops.par_sort_by_key(|(timestamp, _)| *timestamp);
                ops
            })
            .collect();

        Ok(ops_for_list)
    }

    fn with_gradient_color_scheme(
        &mut self,
        start: String,
        end: String,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        let mut cloned = self.clone();
        cloned.gradient = Some(Arc::new(
            colorgrad::CustomGradient::new()
                .html_colors(&[start.as_str(), end.as_str()])
                .build()
                .map_err(|err| error(format!("failed to create a gradient: {}", err)))?,
        ));

        return Ok(cloned);
    }

    fn graph_kind(&self) -> Option<GraphKind> {
        self.kind.or_else(|| {
            if !self.allocation_lists.is_empty() {
                Some(GraphKind::Allocation(AllocationGraphKind::MemoryUsage))
            } else if !self.map_lists.is_empty() {
                Some(GraphKind::Map(MapGraphKind::RSS))
            } else {
                None
            }
        })
    }

    fn data(&self) -> Option<DataRef> {
        match self.graph_kind()? {
            GraphKind::Allocation(..) => Some(self.allocation_lists[0].data.clone()),
            GraphKind::Map(..) => Some(self.map_lists[0].data.clone()),
        }
    }

    fn save_to_string_impl(
        &self,
        xs: &[u64],
        datapoints_for_ops: &[Vec<(u64, u64)>],
        labels: &[Option<String>],
    ) -> Result<String, String> {
        let data = self.data().ok_or_else(|| "empty graph".to_owned())?;

        let mut x_min = if self.trim_left {
            xs.first().copied().unwrap_or(0)
        } else {
            data.initial_timestamp.as_usecs()
        };

        let mut x_max = if self.trim_right {
            xs.last().copied().unwrap_or(0)
        } else {
            data.last_timestamp.as_usecs()
        };

        if let Some(start_at) = self.start_at {
            x_min = std::cmp::max(x_min, (data.initial_timestamp + start_at.0).as_usecs());
        }

        if let Some(end_at) = self.end_at {
            x_max = (data.initial_timestamp + end_at.0).as_usecs();
        }

        x_max = std::cmp::max(x_min, x_max);

        let datapoints_for_ops: Vec<_> = if x_min > xs.first().copied().unwrap_or(0)
            || x_max < xs.last().copied().unwrap_or(0)
        {
            datapoints_for_ops
                .iter()
                .map(|list| {
                    let list = list.as_slice();
                    let list = &list[list.iter().take_while(|&&(x, _)| x < x_min).count()..];
                    let list = &list
                        [..list.len() - list.iter().rev().take_while(|&&(x, _)| x > x_max).count()];
                    list
                })
                .collect()
        } else {
            datapoints_for_ops
                .iter()
                .map(|list| list.as_slice())
                .collect()
        };

        let mut max_usage = 0;
        for &datapoints in &datapoints_for_ops {
            for (_, value) in datapoints {
                max_usage = std::cmp::max(max_usage, *value);
            }
        }

        // This is a dirty hack, but it works.
        thread_local! {
            static SCALE_X: Cell< (u64, u64) > = Cell::new( (0, 0) );
            static SCALE_X_TOTAL: Cell< (u64, u64) > = Cell::new( (0, 0) );
            static SCALE_Y: Cell< (u64, u64) > = Cell::new( (0, 0) );
            static KIND: Cell< GraphKind > = Cell::new( GraphKind::Allocation( AllocationGraphKind::MemoryUsage ) );
        }

        macro_rules! impl_ranged {
            ($kind:ty) => {
                impl Ranged for $kind {
                    type FormatOption = plotters::coord::ranged1d::NoDefaultFormatting;
                    type ValueType = u64;
                    fn map(&self, value: &Self::ValueType, limit: (i32, i32)) -> i32 {
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

                        limit.0
                            + (screen_range as f64 * data_relative_position + 1e-3).floor() as i32
                    }

                    fn key_points<Hint: plotters::coord::ranged1d::KeyPointHint>(
                        &self,
                        hint: Hint,
                    ) -> Vec<Self::ValueType> {
                        gen_keypoints((self.0, self.1), hint.max_num_points())
                    }

                    fn range(&self) -> std::ops::Range<Self::ValueType> {
                        self.0..self.1
                    }
                }
            };
        }

        struct SizeRange(u64, u64);

        impl plotters::coord::ranged1d::ValueFormatter<u64> for SizeRange {
            fn format(value: &u64) -> String {
                SCALE_Y.with(|cell| {
                    let (min, max) = cell.get();

                    if max < 1024 {
                        format!("{}", value)
                    } else {
                        match KIND.with(|cell| cell.get()) {
                            GraphKind::Allocation(AllocationGraphKind::MemoryUsage)
                            | GraphKind::Map(_) => {
                                let (unit, multiplier) = {
                                    if max < 1024 * 1024 {
                                        ("KB", 1024)
                                    } else {
                                        ("MB", 1024 * 1024)
                                    }
                                };

                                if max - min <= (10 * multiplier) {
                                    format!("{:.02} {}", *value as f64 / multiplier as f64, unit)
                                } else if max - min <= (100 * multiplier) {
                                    format!("{:.01} {}", *value as f64 / multiplier as f64, unit)
                                } else {
                                    format!("{} {}", value / multiplier, unit)
                                }
                            }
                            GraphKind::Allocation(
                                AllocationGraphKind::LiveAllocations
                                | AllocationGraphKind::NewAllocations
                                | AllocationGraphKind::Deallocations,
                            ) => {
                                let (unit, multiplier) = {
                                    if max < 1000 * 1000 {
                                        ("K", 1000)
                                    } else {
                                        ("M", 1000 * 1000)
                                    }
                                };

                                if max - min <= (10 * multiplier) {
                                    format!("{:.02} {}", *value as f64 / multiplier as f64, unit)
                                } else if max - min <= (100 * multiplier) {
                                    format!("{:.01} {}", *value as f64 / multiplier as f64, unit)
                                } else {
                                    format!("{} {}", value / multiplier, unit)
                                }
                            }
                        }
                    }
                })
            }
        }

        impl_ranged!(SizeRange);

        struct TimeRange(u64, u64);

        impl plotters::coord::ranged1d::ValueFormatter<u64> for TimeRange {
            fn format(value: &u64) -> String {
                use chrono::prelude::*;

                SCALE_X.with(|cell| {
                    let (min, max) = cell.get();
                    debug_assert!(*value >= min);

                    let start = to_chrono(min);
                    let end = to_chrono(max);
                    let ts = to_chrono(*value);
                    if start.year() == end.year()
                        && start.month() == end.month()
                        && start.day() == end.day()
                    {
                        format!("{:02}:{:02}:{:02}", ts.hour(), ts.minute(), ts.second())
                    } else if start.year() == end.year() && start.month() == end.month() {
                        format!(
                            "{:02} {:02}:{:02}:{:02}",
                            ts.day(),
                            ts.hour(),
                            ts.minute(),
                            ts.second()
                        )
                    } else if start.year() == end.year() {
                        format!(
                            "{:02}-{:02} {:02}:{:02}:{:02}",
                            ts.month(),
                            ts.day(),
                            ts.hour(),
                            ts.minute(),
                            ts.second()
                        )
                    } else {
                        format!(
                            "{}-{:02}-{:02} {:02}:{:02}:{:02}",
                            ts.year(),
                            ts.month(),
                            ts.day(),
                            ts.hour(),
                            ts.minute(),
                            ts.second()
                        )
                    }
                })
            }
        }

        impl_ranged!(TimeRange);

        struct TimeRangeOffset(u64, u64);

        impl plotters::coord::ranged1d::ValueFormatter<u64> for TimeRangeOffset {
            fn format(value: &u64) -> String {
                SCALE_X_TOTAL.with(|cell| {
                    let (min, _max) = cell.get();
                    debug_assert!(*value >= min);
                    let relative = *value - min;
                    let relative_s = relative / 1_000_000;

                    if relative == 0 {
                        format!("0")
                    } else if relative < 1_000 {
                        format!("+{}us", relative)
                    } else if relative < 1_000_000 {
                        format!("+{}ms", relative / 1_000)
                    } else if relative < 60_000_000 {
                        format!("+{}s", relative / 1_000_000)
                    } else {
                        let rh = relative_s / 3600;
                        let rm = (relative_s - rh * 3600) / 60;
                        let rs = relative_s - rh * 3600 - rm * 60;
                        return format!("+{:02}:{:02}:{:02}", rh, rm, rs);
                    }
                })
            }
        }

        impl_ranged!(TimeRangeOffset);

        let graph_kind = self
            .graph_kind()
            .unwrap_or(GraphKind::Allocation(AllocationGraphKind::MemoryUsage));

        SCALE_X.with(|cell| cell.set((x_min, x_max + 1)));
        SCALE_X_TOTAL.with(|cell| {
            cell.set((
                data.initial_timestamp.as_usecs(),
                data.last_timestamp.as_usecs() + 1,
            ))
        });
        SCALE_Y.with(|cell| cell.set((0, (max_usage + 1) as u64)));
        KIND.with(|cell| cell.set(graph_kind));

        let mut output = String::new();
        use plotters::prelude::*;
        let root = SVGBackend::with_string(&mut output, (1024, 768)).into_drawing_area();
        root.fill(&WHITE)
            .map_err(|error| format!("failed to fill the graph with white: {}", error))?;

        let mut chart = ChartBuilder::on(&root);
        let mut chart = &mut chart;
        if !self.without_axes {
            chart = chart
                .margin((1).percent())
                .set_label_area_size(LabelAreaPosition::Left, 70)
                .margin_right(50)
                .set_label_area_size(LabelAreaPosition::Bottom, 60)
                .set_label_area_size(LabelAreaPosition::Top, 60)
        };

        let mut chart = chart
            .build_cartesian_2d(
                TimeRange(x_min, x_max + 1),
                SizeRange(0, (max_usage + 1) as u64),
            )
            .map_err(|error| format!("failed to construct the chart builder: {}", error))?
            .set_secondary_coord(
                TimeRangeOffset(x_min, x_max + 1),
                SizeRange(0, (max_usage + 1) as u64),
            );

        let mut colors = Vec::new();
        if let Some(ref gradient) = self.gradient {
            let step = 1.0 / (std::cmp::max(1, datapoints_for_ops.len()) - 1) as f64;
            for index in 0..datapoints_for_ops.len() {
                let position = index as f64 * step;
                let color = gradient.at(position);
                let color_rgb = color.to_rgba8();
                colors.push(
                    RGBColor(color_rgb[0], color_rgb[1], color_rgb[2])
                        .to_rgba()
                        .mix(color.to_linear_rgba().3),
                );
            }
        } else {
            for index in 0..datapoints_for_ops.len() {
                colors.push(Palette99::pick(index).to_rgba());
            }
        }

        for ((datapoints, label), color) in datapoints_for_ops
            .iter()
            .zip(labels.iter())
            .rev()
            .zip(colors.into_iter().rev())
        {
            let series = chart
                .draw_series(
                    AreaSeries::new(
                        datapoints
                            .iter()
                            .map(|&(x, y)| (x, y as u64))
                            .chain(std::iter::once((
                                x_max,
                                datapoints.last().copied().map(|(_, y)| y).unwrap_or(0),
                            ))),
                        0_u64,
                        color,
                    )
                    .border_style(color.stroke_width(1)),
                )
                .map_err(|error| format!("failed to draw a series: {}", error))?;

            if let Some(label) = label {
                if datapoints.is_empty() && self.hide_empty || self.without_legend {
                    continue;
                }

                series.label(label).legend(move |(x, y)| {
                    Rectangle::new([(x, y - 5), (x + 10, y + 5)], color.filled())
                });
            }
        }

        let mut mesh = chart.configure_mesh();
        let mut mesh = &mut mesh;
        if !self.without_axes {
            let label = match graph_kind {
                GraphKind::Allocation(AllocationGraphKind::MemoryUsage) => "Memory usage",
                GraphKind::Allocation(AllocationGraphKind::LiveAllocations) => "Live allocations",
                GraphKind::Allocation(AllocationGraphKind::NewAllocations) => "New allocations",
                GraphKind::Allocation(AllocationGraphKind::Deallocations) => "Deallocations",
                GraphKind::Map(MapGraphKind::RSS) => "RSS",
                GraphKind::Map(MapGraphKind::AddressSpace) => "Address space",
            };
            mesh = mesh.x_desc("Time").y_desc(label);
        }

        if self.without_grid {
            mesh = mesh.disable_mesh();
        }

        mesh.draw()
            .map_err(|error| format!("failed to draw the mesh: {}", error))?;

        if !self.without_axes {
            chart
                .configure_secondary_axes()
                .draw()
                .map_err(|error| format!("failed to draw the secondary axes: {}", error))?;
        }

        if labels.iter().any(|label| label.is_some()) && !self.without_legend {
            chart
                .configure_series_labels()
                .background_style(&WHITE.mix(0.75))
                .border_style(&BLACK)
                .position(SeriesLabelPosition::UpperLeft)
                .draw()
                .map_err(|error| format!("failed to draw the legend: {}", error))?;
        }

        root.present()
            .map_err(|error| format!("failed to present the graph: {}", error))?;
        std::mem::drop(chart);
        std::mem::drop(root);

        Ok(output)
    }

    fn save_to_string(&mut self) -> Result<String, Box<rhai::EvalAltResult>> {
        (|| {
            {
                if self.cached_datapoints.is_none() {
                    let (xs, datapoints_for_ops) = match self.graph_kind() {
                        Some(GraphKind::Allocation(kind)) => {
                            let ops_for_list = self.generate_allocation_ops()?;
                            prepare_allocation_graph_datapoints(
                                &self.allocation_lists[0].data,
                                &ops_for_list,
                                kind,
                            )
                        }
                        Some(GraphKind::Map(kind)) => {
                            let ops_for_list = self.generate_map_ops()?;
                            prepare_map_graph_datapoints(&ops_for_list, kind)
                        }
                        None => Default::default(),
                    };

                    self.cached_datapoints = Some(Arc::new((xs, datapoints_for_ops)));
                }

                let cached = self.cached_datapoints.as_ref().unwrap();
                self.save_to_string_impl(&cached.0, &cached.1, &self.labels)
            }
            .map_err(|error| {
                Box::new(rhai::EvalAltResult::from(format!(
                    "failed to generate a graph: {}",
                    error
                )))
            })
        })()
    }

    fn save(
        &mut self,
        env: &mut dyn Environment,
        path: String,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        let data = self.save_to_string()?;
        env.file_write(&path, FileKind::Svg, data.as_bytes())?;
        Ok(self.clone())
    }

    fn save_each_series_as_graph(
        &mut self,
        env: &mut dyn Environment,
        mut path: String,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        env.mkdir_p(&path)?;
        if path == "." {
            path = "".into();
        } else if !path.ends_with('/') {
            path.push('/');
        }

        match self.graph_kind() {
            Some(GraphKind::Allocation(kind)) => {
                let ops_for_list = self.generate_allocation_ops()?;
                for (index, (ops, label)) in
                    ops_for_list.into_iter().zip(self.labels.iter()).enumerate()
                {
                    let (xs, datapoints_for_ops) = prepare_allocation_graph_datapoints(
                        &self.allocation_lists[0].data,
                        &[ops],
                        kind,
                    );
                    let data =
                        self.save_to_string_impl(&xs, &datapoints_for_ops, &[label.clone()])?;

                    let file_path = if let Some(label) = label {
                        format!("{}{}.svg", path, label)
                    } else {
                        format!("{}Series #{}.svg", path, index)
                    };

                    env.file_write(&file_path, FileKind::Svg, data.as_bytes())?;
                }
            }
            Some(GraphKind::Map(kind)) => {
                let ops_for_list = self.generate_map_ops()?;
                for (index, (ops, label)) in
                    ops_for_list.into_iter().zip(self.labels.iter()).enumerate()
                {
                    let (xs, datapoints_for_ops) = prepare_map_graph_datapoints(&[ops], kind);
                    let data =
                        self.save_to_string_impl(&xs, &datapoints_for_ops, &[label.clone()])?;

                    let file_path = if let Some(label) = label {
                        format!("{}{}.svg", path, label)
                    } else {
                        format!("{}Series #{}.svg", path, index)
                    };

                    env.file_write(&file_path, FileKind::Svg, data.as_bytes())?;
                }
            }
            None => {}
        };

        Ok(self.clone())
    }

    fn save_each_series_as_flamegraph(
        &mut self,
        env: &mut dyn Environment,
        mut path: String,
    ) -> Result<Self, Box<rhai::EvalAltResult>> {
        if !self
            .graph_kind()
            .map(|kind| kind.is_for_allocations())
            .unwrap_or(true)
        {
            return Err(error("only allocation graphs can be saved as a flamegraph"));
        }

        env.mkdir_p(&path)?;
        if path == "." {
            path = "".into();
        } else if !path.ends_with('/') {
            path.push('/');
        }

        let ops_for_list = self.generate_allocation_ops()?;
        for (index, ((list, ops), label)) in self
            .allocation_lists
            .iter()
            .zip(ops_for_list)
            .zip(self.labels.iter())
            .enumerate()
        {
            let ids: HashSet<_> = ops.into_iter().map(|op| op.id()).collect();
            let mut list = AllocationList {
                data: list.data.clone(),
                allocation_ids: Some(Arc::new(ids.into_iter().collect())),
                filter: None,
            };

            let file_path = if let Some(label) = label {
                format!("{}{}.svg", path, label)
            } else {
                format!("{}Series #{}.svg", path, index)
            };

            list.save_as_flamegraph(env, file_path)?;
        }
        Ok(self.clone())
    }
}

fn load(path: String) -> Result<Arc<Data>, Box<rhai::EvalAltResult>> {
    info!("Loading {:?}...", path);
    let fp = File::open(&path)
        .map_err(|error| rhai::EvalAltResult::from(format!("failed to open '{}': {}", path, error)))
        .map_err(Box::new)?;

    let debug_symbols: &[PathBuf] = &[];
    let data = Loader::load_from_stream(fp, debug_symbols)
        .map_err(|error| rhai::EvalAltResult::from(format!("failed to load '{}': {}", path, error)))
        .map_err(Box::new)?;

    Ok(Arc::new(data))
}

pub fn error(message: impl Into<String>) -> Box<rhai::EvalAltResult> {
    Box::new(rhai::EvalAltResult::from(message.into()))
}

#[derive(Copy, Clone)]
pub enum FileKind {
    Svg,
}

pub struct Engine {
    inner: rhai::Engine,
}

#[derive(Default)]
pub struct EngineArgs {
    pub argv: Vec<String>,
    pub data: Option<Arc<Data>>,
    pub allocation_ids: Option<Arc<Vec<AllocationId>>>,
    pub map_ids: Option<Arc<Vec<MapId>>>,
}

pub trait Environment {
    fn println(&mut self, message: &str);
    fn mkdir_p(&mut self, path: &str) -> Result<(), Box<rhai::EvalAltResult>>;
    fn chdir(&mut self, path: &str) -> Result<(), Box<rhai::EvalAltResult>>;
    fn file_write(
        &mut self,
        path: &str,
        kind: FileKind,
        contents: &[u8],
    ) -> Result<(), Box<rhai::EvalAltResult>>;
    fn exit(&mut self, errorcode: Option<i32>) -> Result<(), Box<rhai::EvalAltResult>> {
        Err(Box::new(rhai::EvalAltResult::Return(
            (errorcode.unwrap_or(0) as i64).into(),
            rhai::Position::NONE,
        )))
    }
    fn load(&mut self, _path: String) -> Result<Arc<Data>, Box<rhai::EvalAltResult>> {
        Err(error("unsupported in this environment"))
    }
}

#[derive(Default)]
pub struct NativeEnvironment {}

impl Environment for NativeEnvironment {
    fn println(&mut self, message: &str) {
        println!("{}", message);
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), Box<rhai::EvalAltResult>> {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("failed to create '{}': {}", path, error).into())
            .map_err(Box::new)
    }

    fn chdir(&mut self, path: &str) -> Result<(), Box<rhai::EvalAltResult>> {
        std::env::set_current_dir(path)
            .map_err(|error| format!("failed to chdir to '{}': {}", path, error).into())
            .map_err(Box::new)
    }

    fn file_write(
        &mut self,
        path: &str,
        _kind: FileKind,
        contents: &[u8],
    ) -> Result<(), Box<rhai::EvalAltResult>> {
        use std::io::Write;

        let mut fp = File::create(&path).map_err(|error| {
            Box::new(rhai::EvalAltResult::from(format!(
                "failed to create {:?}: {}",
                path, error
            )))
        })?;

        fp.write_all(contents).map_err(|error| {
            Box::new(rhai::EvalAltResult::from(format!(
                "failed to write to {:?}: {}",
                path, error
            )))
        })?;

        Ok(())
    }

    fn exit(&mut self, errorcode: Option<i32>) -> Result<(), Box<rhai::EvalAltResult>> {
        std::process::exit(errorcode.unwrap_or(0));
    }

    fn load(&mut self, path: String) -> Result<Arc<Data>, Box<rhai::EvalAltResult>> {
        load(path)
    }
}

fn to_string(value: rhai::plugin::Dynamic) -> String {
    if value.is::<String>() {
        value.cast::<String>()
    } else if value.is::<i64>() {
        value.cast::<i64>().to_string()
    } else if value.is::<u64>() {
        value.cast::<u64>().to_string()
    } else if value.is::<bool>() {
        value.cast::<bool>().to_string()
    } else if value.is::<f64>() {
        value.cast::<f64>().to_string()
    } else if value.is::<Duration>() {
        value.cast::<Duration>().decompose().to_string()
    } else if value.is::<Option<Duration>>() {
        if let Some(duration) = value.cast::<Option<Duration>>() {
            format!("Some({})", duration.decompose().to_string())
        } else {
            "None".into()
        }
    } else if value.is::<AllocationList>() {
        let mut value = value.cast::<AllocationList>();
        format!("{} allocation(s)", value.len())
    } else if value.is::<MapList>() {
        let mut value = value.cast::<MapList>();
        format!("{} map(s)", value.len())
    } else if value.is::<Backtrace>() {
        value.cast::<Backtrace>().to_string()
    } else if value.is::<Option<Backtrace>>() {
        if let Some(backtrace) = value.cast::<Option<Backtrace>>() {
            backtrace.to_string()
        } else {
            "None".into()
        }
    } else {
        value.type_name().into()
    }
}

fn format(fmt: &str, args: &[&str]) -> Result<String, Box<rhai::EvalAltResult>> {
    let mut output = String::with_capacity(fmt.len());
    let mut tmp = String::new();
    let mut in_interpolation = false;
    let mut current_arg = 0;
    for ch in fmt.chars() {
        if in_interpolation {
            if tmp.is_empty() && ch == '{' {
                in_interpolation = false;
                output.push(ch);
                continue;
            }
            if ch == '}' {
                in_interpolation = false;
                if tmp.is_empty() {
                    if current_arg >= args.len() {
                        return Err(error("too many positional arguments in the format string"));
                    }
                    output.push_str(args[current_arg]);
                    current_arg += 1;
                } else {
                    let position: Result<usize, _> = tmp.parse();
                    if let Ok(position) = position {
                        tmp.clear();
                        if position >= args.len() {
                            return Err(error(format!(
                                "invalid reference to positional argument {}",
                                position
                            )));
                        }
                        output.push_str(args[position]);
                    } else {
                        return Err(error(format!("malformed positional argument \"{}\"", tmp)));
                    }
                }
                continue;
            }
            tmp.push(ch);
        } else {
            if ch == '{' {
                in_interpolation = true;
                continue;
            }
            output.push(ch);
        }
    }

    if in_interpolation {
        return Err(error("malformed format string"));
    }

    Ok(output)
}

impl Engine {
    pub fn new(env: Arc<Mutex<dyn Environment>>, args: EngineArgs) -> Self {
        use rhai::packages::Package;

        let mut engine = rhai::Engine::new_raw();
        engine.register_global_module(rhai::packages::ArithmeticPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicArrayPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicFnPackage::new().as_shared_module());
        engine
            .register_global_module(rhai::packages::BasicIteratorPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicMapPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicMathPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::BasicStringPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::LogicPackage::new().as_shared_module());
        engine.register_global_module(rhai::packages::MoreStringPackage::new().as_shared_module());

        let argv = args.argv;

        // Utility functions.
        engine.register_fn("dirname", dirname);
        engine.register_fn("h", |value: i64| Duration::from_secs(value as u64 * 3600));
        engine.register_fn("h", |value: f64| {
            Duration::from_usecs((value * 3600.0 * 1_000_000.0) as u64)
        });
        engine.register_fn("m", |value: i64| Duration::from_secs(value as u64 * 60));
        engine.register_fn("m", |value: f64| {
            Duration::from_usecs((value * 60.0 * 1_000_000.0) as u64)
        });
        engine.register_fn("s", |value: i64| Duration::from_secs(value as u64));
        engine.register_fn("s", |value: f64| {
            Duration::from_secs((value * 1_000_000.0) as u64)
        });
        engine.register_fn("ms", |value: i64| Duration::from_msecs(value as u64));
        engine.register_fn("ms", |value: f64| {
            Duration::from_usecs((value * 1_000.0) as u64)
        });
        engine.register_fn("us", |value: i64| Duration::from_usecs(value as u64));
        engine.register_fn("us", |value: f64| Duration::from_usecs(value as u64));
        engine.register_fn("*", |lhs: Duration, rhs: i64| -> Duration {
            Duration(lhs.0 * rhs as f64)
        });
        engine.register_fn("*", |lhs: i64, rhs: Duration| -> Duration {
            Duration(rhs.0 * lhs as f64)
        });
        engine.register_fn("*", |lhs: Duration, rhs: f64| -> Duration {
            Duration(lhs.0 * rhs as f64)
        });
        engine.register_fn("*", |lhs: f64, rhs: Duration| -> Duration {
            Duration(rhs.0 * lhs as f64)
        });
        engine.register_fn("+", |lhs: Duration, rhs: Duration| -> Duration {
            Duration(lhs.0 + rhs.0)
        });
        engine.register_fn("-", |lhs: Duration, rhs: Duration| -> Duration {
            Duration(lhs.0 - rhs.0)
        });
        engine.register_fn("kb", |value: i64| value * 1000);
        engine.register_fn("mb", |value: i64| value * 1000 * 1000);
        engine.register_fn("gb", |value: i64| value * 1000 * 1000 * 1000);
        engine.register_fn("info", |message: &str| info!("{}", message));
        engine.register_type::<Duration>();
        engine.register_fn("argv", move || -> rhai::Array {
            argv.iter().cloned().map(rhai::Dynamic::from).collect()
        });

        {
            let env = env.clone();
            engine.register_result_fn("mkdir_p", move |path: &str| env.lock().mkdir_p(path));
        }
        {
            let env = env.clone();
            engine.register_result_fn("chdir", move |path: &str| env.lock().chdir(path));
        }
        {
            let env = env.clone();
            engine.register_result_fn("exit", move |errorcode: i64| {
                env.lock().exit(Some(errorcode as i32))
            });
        }
        {
            let env = env.clone();
            engine.register_result_fn("exit", move || env.lock().exit(None));
        }

        // DSL functions.
        engine.register_type::<DataRef>();
        engine.register_type::<Allocation>();
        engine.register_type::<AllocationList>();
        engine.register_type::<AllocationGroupList>();
        engine.register_type::<Map>();
        engine.register_type::<MapList>();
        engine.register_type::<Backtrace>();
        engine.register_type::<Graph>();
        engine.register_fn("graph", Graph::new);
        engine.register_result_fn("add", Graph::add);
        engine.register_result_fn("add", Graph::add_with_label);
        engine.register_result_fn("add", Graph::add_group);
        engine.register_result_fn("add", Graph::add_maps);
        engine.register_result_fn("add", Graph::add_maps_with_label);
        engine.register_fn("trim_left", Graph::trim_left);
        engine.register_fn("trim_right", Graph::trim_right);
        engine.register_fn("trim", Graph::trim);
        engine.register_fn("start_at", Graph::start_at);
        engine.register_fn("end_at", Graph::end_at);
        engine.register_fn("only_non_empty_series", Graph::only_non_empty_series);
        engine.register_fn("without_legend", Graph::without_legend);
        engine.register_fn("without_axes", Graph::without_axes);
        engine.register_fn("without_grid", Graph::without_grid);
        engine.register_result_fn("show_memory_usage", Graph::show_memory_usage);
        engine.register_result_fn("show_live_allocations", Graph::show_live_allocations);
        engine.register_result_fn("show_new_allocations", Graph::show_new_allocations);
        engine.register_result_fn("show_deallocations", Graph::show_deallocations);
        engine.register_result_fn("show_rss", Graph::show_rss);
        engine.register_result_fn("show_address_space", Graph::show_address_space);

        engine.register_result_fn(
            "with_gradient_color_scheme",
            Graph::with_gradient_color_scheme,
        );
        engine.register_fn("allocations", DataRef::allocations);
        engine.register_fn("maps", DataRef::maps);
        engine.register_fn("runtime", |data: &mut DataRef| {
            Duration(data.0.last_timestamp - data.0.initial_timestamp)
        });

        engine.register_fn("strip", |backtrace: &mut Backtrace| {
            let mut cloned = backtrace.clone();
            cloned.strip = true;
            cloned
        });

        fn set_max<T>(target: &mut Option<T>, value: T)
        where
            T: PartialOrd,
        {
            if let Some(target) = target.as_mut() {
                if *target < value {
                    *target = value;
                }
            } else {
                *target = Some(value);
            }
        }

        fn set_min<T>(target: &mut Option<T>, value: T)
        where
            T: PartialOrd,
        {
            if let Some(target) = target.as_mut() {
                if *target > value {
                    *target = value;
                }
            } else {
                *target = Some(value);
            }
        }

        fn gather_backtrace_ids(
            set: &mut HashSet<BacktraceId>,
            arg: rhai::Dynamic,
        ) -> Result<(), Box<rhai::EvalAltResult>> {
            if let Some(id) = arg.clone().try_cast::<i64>() {
                set.insert(BacktraceId::new(id as u32));
            } else if let Some(obj) = arg.clone().try_cast::<Backtrace>() {
                set.insert(obj.id);
            } else if let Some(mut obj) = arg.clone().try_cast::<AllocationList>() {
                let data = obj.data.clone();
                obj.apply_filter();
                set.par_extend(obj.unfiltered_ids_par_iter().map(|allocation_id| {
                    let allocation = data.get_allocation(allocation_id);
                    allocation.backtrace
                }));
            } else if let Some(mut obj) = arg.clone().try_cast::<MapList>() {
                let data = obj.data.clone();
                obj.apply_filter();
                set.par_extend(obj.unfiltered_ids_par_iter().flat_map(|id| {
                    let map = data.get_map(id);
                    map.source.map(|source| source.backtrace)
                }));
            } else if let Some(obj) = arg.clone().try_cast::<AllocationGroupList>() {
                let data = obj.data.clone();
                for group in obj.groups.iter() {
                    for allocation_id in group.allocation_ids.iter() {
                        let allocation = data.get_allocation(*allocation_id);
                        set.insert(allocation.backtrace);
                    }
                }
            } else if let Some(obj) = arg.clone().try_cast::<rhai::Array>() {
                for subobj in obj {
                    gather_backtrace_ids(set, subobj)?;
                }
            } else {
                let error = error( format!( "expected a raw backtrace ID, 'Backtrace' object, 'AllocationList' object, 'MapList' object, or an array of any of them, got {}", arg.type_name() ) );
                return Err(error);
            }

            Ok(())
        }

        fn gather_map_ids(
            set: &mut HashSet<MapId>,
            arg: rhai::Dynamic,
        ) -> Result<(), Box<rhai::EvalAltResult>> {
            if let Some(id) = arg.clone().try_cast::<i64>() {
                set.insert(MapId(id as u64));
            } else if let Some(obj) = arg.clone().try_cast::<Map>() {
                set.insert(obj.id);
            } else if let Some(mut obj) = arg.clone().try_cast::<MapList>() {
                obj.apply_filter();
                set.par_extend(obj.unfiltered_ids_par_iter());
            } else if let Some(obj) = arg.clone().try_cast::<rhai::Array>() {
                for subobj in obj {
                    gather_map_ids(set, subobj)?;
                }
            } else {
                let error = error( format!( "expected a raw map ID, 'Map' object, 'MapList' object, or an array of any of them, got {}", arg.type_name() ) );
                return Err(error);
            }

            Ok(())
        }

        macro_rules! register_filter {
            ($ty_name:ident, $setter:ident, $field:ident.$name:ident, $src_ty:ty => $dst_ty:ty) => {
                engine.register_fn(stringify!($name), |list: &mut $ty_name, value: $src_ty| {
                    list.add_filter(|filter| $setter(&mut filter.$field.$name, value as $dst_ty))
                });
            };

            ($ty_name:ident, $setter:ident, $name:ident, $src_ty:ty => $dst_ty:ty) => {
                engine.register_fn(stringify!($name), |list: &mut $ty_name, value: $src_ty| {
                    list.add_filter(|filter| $setter(&mut filter.$name, value as $dst_ty))
                });
            };

            ($ty_name:ident, $field:ident.$name:ident, bool) => {
                engine.register_fn(stringify!($name), |list: &mut $ty_name| {
                    list.add_filter(|filter| filter.$field.$name = true)
                });
            };

            ($ty_name:ident, $name:ident, bool) => {
                engine.register_fn(stringify!($name), |list: &mut $ty_name| {
                    list.add_filter(|filter| filter.$name = true)
                });
            };

            ($ty_name:ident, $setter:ident, $field:ident.$name:ident, $ty:ty) => {
                engine.register_fn(stringify!($name), |list: &mut $ty_name, value: $ty| {
                    list.add_filter(|filter| $setter(&mut filter.$field.$name, value as $ty))
                });
            };

            ($ty_name:ident, $setter:ident, $name:ident, $ty:ty) => {
                engine.register_fn(stringify!($name), |list: &mut $ty_name, value: $ty| {
                    list.add_filter(|filter| $setter(&mut filter.$name, value as $ty))
                });
            };
        }

        register_filter!( AllocationList, set_max, only_first_size_larger_or_equal, i64 => u64 );
        register_filter!( AllocationList, set_min, only_first_size_smaller_or_equal, i64 => u64 );
        register_filter!( AllocationList, set_max, only_first_size_larger, i64 => u64 );
        register_filter!( AllocationList, set_min, only_first_size_smaller, i64 => u64 );
        register_filter!( AllocationList, set_max, only_last_size_larger_or_equal, i64 => u64 );
        register_filter!( AllocationList, set_min, only_last_size_smaller_or_equal, i64 => u64 );
        register_filter!( AllocationList, set_max, only_last_size_larger, i64 => u64 );
        register_filter!( AllocationList, set_min, only_last_size_smaller, i64 => u64 );
        register_filter!( AllocationList, set_max, only_chain_length_at_least, i64 => u32 );
        register_filter!( AllocationList, set_min, only_chain_length_at_most, i64 => u32 );
        register_filter!(
            AllocationList,
            set_max,
            only_chain_alive_for_at_least,
            Duration
        );
        register_filter!(
            AllocationList,
            set_min,
            only_chain_alive_for_at_most,
            Duration
        );
        register_filter!( AllocationList, set_max, only_position_in_chain_at_least, i64 => u32 );
        register_filter!( AllocationList, set_min, only_position_in_chain_at_most, i64 => u32 );

        register_filter!( AllocationList, set_max, only_group_allocations_at_least, i64 => usize );
        register_filter!( AllocationList, set_min, only_group_allocations_at_most, i64 => usize );
        register_filter!(
            AllocationList,
            set_max,
            only_group_interval_at_least,
            Duration
        );
        register_filter!(
            AllocationList,
            set_min,
            only_group_interval_at_most,
            Duration
        );
        register_filter!(
            AllocationList,
            set_max,
            only_group_max_total_usage_first_seen_at_least,
            Duration
        );
        register_filter!(
            AllocationList,
            set_min,
            only_group_max_total_usage_first_seen_at_most,
            Duration
        );

        engine.register_fn(
            "only_group_leaked_allocations_at_least",
            |list: &mut AllocationList, value: f64| {
                list.add_filter_once(
                    |filter| filter.only_group_leaked_allocations_at_least.is_some(),
                    |filter| {
                        filter.only_group_leaked_allocations_at_least =
                            Some(NumberOrFractionOfTotal::Fraction(value))
                    },
                )
            },
        );
        engine.register_fn(
            "only_group_leaked_allocations_at_least",
            |list: &mut AllocationList, value: i64| {
                list.add_filter_once(
                    |filter| filter.only_group_leaked_allocations_at_least.is_some(),
                    |filter| {
                        filter.only_group_leaked_allocations_at_least =
                            Some(NumberOrFractionOfTotal::Number(value as u64))
                    },
                )
            },
        );
        engine.register_fn(
            "only_group_leaked_allocations_at_most",
            |list: &mut AllocationList, value: f64| {
                list.add_filter_once(
                    |filter| filter.only_group_leaked_allocations_at_most.is_some(),
                    |filter| {
                        filter.only_group_leaked_allocations_at_most =
                            Some(NumberOrFractionOfTotal::Fraction(value))
                    },
                )
            },
        );
        engine.register_fn(
            "only_group_leaked_allocations_at_most",
            |list: &mut AllocationList, value: i64| {
                list.add_filter_once(
                    |filter| filter.only_group_leaked_allocations_at_most.is_some(),
                    |filter| {
                        filter.only_group_leaked_allocations_at_most =
                            Some(NumberOrFractionOfTotal::Number(value as u64))
                    },
                )
            },
        );

        register_filter!(AllocationList, only_chain_leaked, bool);
        register_filter!(AllocationList, only_ptmalloc_mmaped, bool);
        register_filter!(AllocationList, only_ptmalloc_not_mmaped, bool);
        register_filter!(AllocationList, only_ptmalloc_from_main_arena, bool);
        register_filter!(AllocationList, only_ptmalloc_not_from_main_arena, bool);
        register_filter!(AllocationList, only_jemalloc, bool);
        register_filter!(AllocationList, only_not_jemalloc, bool);

        engine.register_fn(
            "only_with_marker",
            |list: &mut AllocationList, value: i64| {
                list.add_filter_once(
                    |filter| filter.only_with_marker.is_some(),
                    |filter| filter.only_with_marker = Some(value as u32),
                )
            },
        );

        engine.register_fn("group_by_backtrace", AllocationList::group_by_backtrace);

        engine.register_fn("only_all_leaked", AllocationGroupList::only_all_leaked);
        engine.register_fn(
            "only_count_at_least",
            AllocationGroupList::only_count_at_least,
        );
        engine.register_fn("len", AllocationGroupList::len);
        engine.register_fn(
            "sort_by_size_ascending",
            AllocationGroupList::sort_by_size_ascending,
        );
        engine.register_fn(
            "sort_by_size_descending",
            AllocationGroupList::sort_by_size_descending,
        );
        engine.register_fn("sort_by_size", AllocationGroupList::sort_by_size_descending);
        engine.register_fn(
            "sort_by_count_ascending",
            AllocationGroupList::sort_by_count_ascending,
        );
        engine.register_fn(
            "sort_by_count_descending",
            AllocationGroupList::sort_by_count_descending,
        );
        engine.register_fn(
            "sort_by_count",
            AllocationGroupList::sort_by_count_descending,
        );
        engine.register_fn("ungroup", AllocationGroupList::ungroup);
        engine.register_indexer_get_result(AllocationGroupList::get);
        engine.register_fn("take", AllocationGroupList::take);
        engine.register_iterator::<AllocationGroupList>();

        engine.register_fn("backtrace", |allocation: &mut Allocation| Backtrace {
            data: allocation.data.clone(),
            id: allocation.data.get_allocation(allocation.id).backtrace,
            strip: false,
        });

        engine.register_fn("backtrace", |map: &mut Map| {
            map.data.get_map(map.id).source.map(|source| Backtrace {
                data: map.data.clone(),
                id: source.backtrace,
                strip: false,
            })
        });

        engine.register_fn("allocated_at", |allocation: &mut Allocation| {
            Duration(
                allocation.data.get_allocation(allocation.id).timestamp
                    - allocation.data.initial_timestamp,
            )
        });

        engine.register_fn("allocated_at", |map: &mut Map| {
            Duration(map.data.get_map(map.id).timestamp - map.data.initial_timestamp)
        });

        engine.register_fn("deallocated_at", |allocation: &mut Allocation| {
            Some(Duration(
                allocation
                    .data
                    .get_allocation(allocation.id)
                    .deallocation
                    .as_ref()?
                    .timestamp
                    - allocation.data.initial_timestamp,
            ))
        });

        engine.register_fn("deallocated_at", |map: &mut Map| {
            Some(Duration(
                map.data.get_map(map.id).deallocation.as_ref()?.timestamp
                    - map.data.initial_timestamp,
            ))
        });

        register_filter!( MapList, set_max, only_peak_rss_at_least, i64 => u64 );
        register_filter!( MapList, set_min, only_peak_rss_at_most, i64 => u64 );
        register_filter!(MapList, only_jemalloc, bool);
        register_filter!(MapList, only_not_jemalloc, bool);
        register_filter!(MapList, only_bytehound, bool);
        register_filter!(MapList, only_not_bytehound, bool);
        register_filter!(MapList, only_readable, bool);
        register_filter!(MapList, only_not_readable, bool);
        register_filter!(MapList, only_writable, bool);
        register_filter!(MapList, only_not_writable, bool);
        register_filter!(MapList, only_executable, bool);
        register_filter!(MapList, only_not_executable, bool);

        let graph_counter = Arc::new(AtomicUsize::new(1));
        let flamegraph_counter = Arc::new(AtomicUsize::new(1));

        fn get_counter(graph_counter: &AtomicUsize) -> usize {
            graph_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        }

        {
            let data = args.data.clone();
            engine.register_result_fn("data", move || {
                if let Some(ref data) = data {
                    Ok(DataRef(data.clone()))
                } else {
                    Err(error("no globally loaded data file"))
                }
            });
        }

        {
            let data = args.data.clone();
            let allocation_ids = args.allocation_ids.clone();
            engine.register_result_fn("allocations", move || {
                if let Some(ref data) = data {
                    Ok(AllocationList {
                        data: DataRef(data.clone()),
                        allocation_ids: allocation_ids.clone(),
                        filter: None,
                    })
                } else {
                    Err(error("no globally loaded allocations"))
                }
            });
        }

        {
            let data = args.data.clone();
            let map_ids = args.map_ids.clone();
            engine.register_result_fn("maps", move || {
                if let Some(ref data) = data {
                    Ok(MapList {
                        data: DataRef(data.clone()),
                        map_ids: map_ids.clone(),
                        filter: None,
                    })
                } else {
                    Err(error("no globally loaded maps"))
                }
            });
        }

        {
            let env = env.clone();
            engine.register_result_fn("load", move |path: String| {
                Ok(DataRef(env.lock().load(path)?))
            });
        }

        {
            let env = env.clone();
            engine.register_result_fn("save", move |graph: &mut Graph, path: String| {
                Graph::save(graph, &mut *env.lock(), path)
            });
        }
        {
            let env = env.clone();
            let graph_counter = graph_counter.clone();
            engine.register_result_fn("save", move |graph: &mut Graph| {
                Graph::save(
                    graph,
                    &mut *env.lock(),
                    format!("Graph #{}.svg", get_counter(&graph_counter)),
                )
            });
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_each_series_as_graph",
                move |graph: &mut Graph, path: String| {
                    Graph::save_each_series_as_graph(graph, &mut *env.lock(), path)
                },
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn("save_each_series_as_graph", move |graph: &mut Graph| {
                Graph::save_each_series_as_graph(graph, &mut *env.lock(), ".".into())
            });
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_each_series_as_flamegraph",
                move |graph: &mut Graph, path: String| {
                    Graph::save_each_series_as_flamegraph(graph, &mut *env.lock(), path)
                },
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_each_series_as_flamegraph",
                move |graph: &mut Graph| {
                    Graph::save_each_series_as_flamegraph(graph, &mut *env.lock(), ".".into())
                },
            );
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_as_flamegraph",
                move |list: &mut AllocationList, path: String| {
                    AllocationList::save_as_flamegraph(list, &mut *env.lock(), path)
                },
            );
        }
        {
            let env = env.clone();
            let flamegraph_counter = flamegraph_counter.clone();
            engine.register_result_fn("save_as_flamegraph", move |list: &mut AllocationList| {
                AllocationList::save_as_flamegraph(
                    list,
                    &mut *env.lock(),
                    format!("Flamegraph #{}.svg", get_counter(&flamegraph_counter)),
                )
            });
        }
        {
            let env = env.clone();
            engine.register_result_fn(
                "save_as_graph",
                move |list: &mut AllocationList, path: String| {
                    AllocationList::save_as_graph(list, &mut *env.lock(), path)
                },
            );
        }
        {
            let env = env.clone();
            let graph_counter = graph_counter.clone();
            engine.register_result_fn("save_as_graph", move |list: &mut AllocationList| {
                AllocationList::save_as_graph(
                    list,
                    &mut *env.lock(),
                    format!("Graph #{}.svg", get_counter(&graph_counter)),
                )
            });
        }

        {
            let env = env.clone();
            engine.register_fn("println", move || {
                env.lock().println("");
            });
        }

        {
            let env = env.clone();
            engine.register_fn("println", move |a0: rhai::plugin::Dynamic| {
                env.lock().println(&to_string(a0));
            });
        }

        {
            let env = env.clone();
            engine.register_result_fn(
                "println",
                move |a0: rhai::plugin::Dynamic, a1: rhai::plugin::Dynamic| {
                    let a0 = to_string(a0);
                    let a1 = to_string(a1);
                    let message = format(&a0, &[&a1])?;
                    env.lock().println(&message);
                    Ok(())
                },
            );
        }

        {
            let env = env.clone();
            engine.register_result_fn(
                "println",
                move |a0: rhai::plugin::Dynamic,
                      a1: rhai::plugin::Dynamic,
                      a2: rhai::plugin::Dynamic| {
                    let a0 = to_string(a0);
                    let a1 = to_string(a1);
                    let a2 = to_string(a2);
                    let message = format(&a0, &[&a1, &a2])?;
                    env.lock().println(&message);
                    Ok(())
                },
            );
        }

        {
            let env = env.clone();
            engine.register_result_fn(
                "println",
                move |a0: rhai::plugin::Dynamic,
                      a1: rhai::plugin::Dynamic,
                      a2: rhai::plugin::Dynamic,
                      a3: rhai::plugin::Dynamic| {
                    let a0 = to_string(a0);
                    let a1 = to_string(a1);
                    let a2 = to_string(a2);
                    let a3 = to_string(a3);
                    let message = format(&a0, &[&a1, &a2, &a3])?;
                    env.lock().println(&message);
                    Ok(())
                },
            );
        }

        macro_rules! register_list {
            ($ty_name:ident) => {{
                engine.register_result_fn( "+", $ty_name::rhai_merge );
                engine.register_result_fn( "-", $ty_name::rhai_substract );
                engine.register_result_fn( "&", $ty_name::rhai_intersect );
                engine.register_fn( "len", $ty_name::len );
                engine.register_indexer_get_result( $ty_name::rhai_get );

                engine.register_result_fn( "only_passing_through_function", |list: &mut $ty_name, regex: String| {
                    let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
                    Ok( list.add_filter_once( |filter| filter.backtrace_filter.only_passing_through_function.is_some(), |filter|
                        filter.backtrace_filter.only_passing_through_function = Some( regex )
                    ))
                });
                engine.register_result_fn( "only_not_passing_through_function", |list: &mut $ty_name, regex: String| {
                    let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
                    Ok( list.add_filter_once( |filter| filter.backtrace_filter.only_not_passing_through_function.is_some(), |filter|
                        filter.backtrace_filter.only_not_passing_through_function = Some( regex )
                    ))
                });
                engine.register_result_fn( "only_passing_through_source", |list: &mut $ty_name, regex: String| {
                    let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
                    Ok( list.add_filter_once( |filter| filter.backtrace_filter.only_passing_through_source.is_some(), |filter|
                        filter.backtrace_filter.only_passing_through_source = Some( regex )
                    ))
                });
                engine.register_result_fn( "only_not_passing_through_source", |list: &mut $ty_name, regex: String| {
                    let regex = regex::Regex::new( &regex ).map_err( |error| Box::new( rhai::EvalAltResult::from( format!( "failed to compile regex: {}", error ) ) ) )?;
                    Ok( list.add_filter_once( |filter| filter.backtrace_filter.only_not_passing_through_source.is_some(), |filter|
                        filter.backtrace_filter.only_not_passing_through_source = Some( regex )
                    ))
                });

                engine.register_result_fn( "only_matching_backtraces", |list: &mut $ty_name, ids: rhai::Dynamic| {
                    let mut set = HashSet::new();
                    gather_backtrace_ids( &mut set, ids )?;

                    if set.len() == 1 && list.unfiltered_ids_ref().is_none() {
                        let id = set.into_iter().next().unwrap();
                        return Ok( $ty_name::create(
                            list.data_ref().clone(),
                            Some( Arc::new( $ty_name::list_by_backtrace( &list.data_ref(), id ) ) ),
                            list.filter_ref().cloned()
                        ));
                    }

                    Ok( list.add_filter( |filter| {
                        if let Some( ref mut existing ) = filter.backtrace_filter.only_matching_backtraces {
                            *existing = existing.intersection( &set ).copied().collect();
                        } else {
                            filter.backtrace_filter.only_matching_backtraces = Some( set );
                        }
                    }) )
                });

                engine.register_result_fn( "only_not_matching_backtraces", |list: &mut $ty_name, ids: rhai::Dynamic| {
                    let mut set = HashSet::new();
                    gather_backtrace_ids( &mut set, ids )?;

                    Ok( list.add_filter( |filter| {
                        filter.backtrace_filter.only_not_matching_backtraces.get_or_insert_with( || HashSet::new() ).extend( set );
                    }))
                });

                engine.register_result_fn( "only_matching_deallocation_backtraces", |list: &mut $ty_name, ids: rhai::Dynamic| {
                    let mut set = HashSet::new();
                    gather_backtrace_ids( &mut set, ids )?;

                    Ok( list.add_filter( |filter| {
                        if let Some( ref mut existing ) = filter.backtrace_filter.only_matching_deallocation_backtraces {
                            *existing = existing.intersection( &set ).copied().collect();
                        } else {
                            filter.backtrace_filter.only_matching_deallocation_backtraces = Some( set );
                        }
                    }))
                });

                engine.register_result_fn( "only_not_matching_deallocation_backtraces", |list: &mut $ty_name, ids: rhai::Dynamic| {
                    let mut set = HashSet::new();
                    gather_backtrace_ids( &mut set, ids )?;

                    Ok( list.add_filter( |filter| {
                        filter.backtrace_filter.only_not_matching_deallocation_backtraces.get_or_insert_with( || HashSet::new() ).extend( set );
                    }))
                });

                register_filter!( $ty_name, set_max, backtrace_filter.only_backtrace_length_at_least, i64 => usize );
                register_filter!( $ty_name, set_min, backtrace_filter.only_backtrace_length_at_most, i64 => usize );
                register_filter!( $ty_name, set_max, common_filter.only_larger_or_equal, i64 => u64 );
                register_filter!( $ty_name, set_min, common_filter.only_smaller_or_equal, i64 => u64 );
                register_filter!( $ty_name, set_max, common_filter.only_larger, i64 => u64 );
                register_filter!( $ty_name, set_min, common_filter.only_smaller, i64 => u64 );
                register_filter!( $ty_name, set_max, common_filter.only_address_at_least, i64 => u64 );
                register_filter!( $ty_name, set_min, common_filter.only_address_at_most, i64 => u64 );

                register_filter!( $ty_name, set_max, common_filter.only_allocated_after_at_least, Duration );
                register_filter!( $ty_name, set_min, common_filter.only_allocated_until_at_most, Duration );
                register_filter!( $ty_name, set_max, common_filter.only_deallocated_after_at_least, Duration );
                register_filter!( $ty_name, set_min, common_filter.only_deallocated_until_at_most, Duration );
                register_filter!( $ty_name, set_max, common_filter.only_alive_for_at_least, Duration );
                register_filter!( $ty_name, set_min, common_filter.only_alive_for_at_most, Duration );

                register_filter!( $ty_name, set_max, common_filter.only_leaked_or_deallocated_after, Duration );

                register_filter!( $ty_name, common_filter.only_leaked, bool );
                register_filter!( $ty_name, common_filter.only_temporary, bool );

                engine.register_result_fn( "only_alive_at", |list: &mut $ty_name, xs: rhai::Array| -> Result< $ty_name, Box< rhai::EvalAltResult > > {
                    let mut xs_cast = Vec::new();
                    for value in xs {
                        if let Some( value ) = value.clone().try_cast::< Duration >() {
                            xs_cast.push( value );
                        } else {
                            return Err( error( format!( "expected an array of 'Duration's, got {}", value.type_name() ) ) );
                        }
                    }

                    Ok( list.add_filter( |filter| filter.common_filter.only_alive_at = xs_cast ) )
                });
            }};
        }

        engine.register_result_fn(
            "only_from_maps",
            |list: &mut AllocationList, ids: rhai::Dynamic| {
                let mut set = HashSet::new();
                gather_map_ids(&mut set, ids)?;

                Ok(list.add_filter(|filter| {
                    if let Some(ref mut existing) = filter.only_from_maps {
                        *existing = existing.intersection(&set).copied().collect();
                    } else {
                        filter.only_from_maps = Some(set);
                    }
                }))
            },
        );

        register_list!(AllocationList);
        register_list!(MapList);

        Engine { inner: engine }
    }

    pub fn run(&self, code: &str) -> Result<Option<EvalOutput>, EvalError> {
        match self.inner.eval::<rhai::plugin::Dynamic>(code) {
            Ok(value) => {
                if value.is::<AllocationList>() {
                    Ok(Some(EvalOutput::AllocationList(
                        value.cast::<AllocationList>(),
                    )))
                } else if value.is::<MapList>() {
                    Ok(Some(EvalOutput::MapList(value.cast::<MapList>())))
                } else {
                    Ok(None)
                }
            }
            Err(error) => {
                let p = error.position();
                Err(EvalError {
                    message: error.to_string(),
                    line: p.line(),
                    column: p.position(),
                })
            }
        }
    }
}

pub enum EvalOutput {
    AllocationList(AllocationList),
    MapList(MapList),
}

#[derive(Debug)]
pub struct EvalError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl From<String> for EvalError {
    fn from(message: String) -> Self {
        EvalError {
            message,
            line: None,
            column: None,
        }
    }
}

impl<'a> From<&'a str> for EvalError {
    fn from(message: &'a str) -> Self {
        message.to_owned().into()
    }
}

pub fn run_script(
    path: &Path,
    data_path: Option<&Path>,
    argv: Vec<String>,
) -> Result<(), std::io::Error> {
    let mut args = EngineArgs {
        argv,
        ..EngineArgs::default()
    };

    if let Some(data_path) = data_path {
        info!("Loading {:?}...", data_path);
        let fp = File::open(&data_path)?;

        let debug_symbols: &[PathBuf] = &[];
        let data = Loader::load_from_stream(fp, debug_symbols)?;
        args.data = Some(Arc::new(data));
    }

    let env = Arc::new(Mutex::new(NativeEnvironment::default()));
    let engine = Engine::new(env, args);

    info!("Running {:?}...", path);
    let result = engine.inner.eval_file::<rhai::plugin::Dynamic>(path.into());
    match result {
        Ok(_) => {}
        Err(error) => {
            error!("{}", error);
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to evaluate the script",
            ));
        }
    }

    Ok(())
}

pub fn run_script_slave(data_path: Option<&Path>) -> Result<(), std::io::Error> {
    let mut args = EngineArgs::default();

    if let Some(data_path) = data_path {
        info!("Loading {:?}...", data_path);
        let fp = File::open(&data_path)?;

        let debug_symbols: &[PathBuf] = &[];
        let data = Loader::load_from_stream(fp, debug_symbols)?;
        args.data = Some(Arc::new(data));
    }

    let env = Arc::new(Mutex::new(VirtualEnvironment::new()));
    let engine = Engine::new(env.clone(), args);
    let mut scope = rhai::Scope::new();
    let mut global_ast: rhai::AST = Default::default();

    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buffer = Vec::new();
    loop {
        use std::io::BufRead;

        buffer.clear();
        match stdin.read_until(0, &mut buffer) {
            Ok(count) => {
                if count == 0 {
                    return Ok(());
                }
            }
            Err(_) => return Ok(()),
        }

        if buffer.ends_with(b"\0") {
            buffer.pop();
        }

        let input = match std::str::from_utf8(&buffer) {
            Ok(input) => input,
            Err(_) => {
                let payload = serde_json::json! {{
                    "kind": "syntax_error",
                    "message": "invalid utf-8"
                }};

                println!("{}", serde_json::to_string(&payload).unwrap());

                let payload = serde_json::json! {{
                    "kind": "idle"
                }};

                println!("{}", serde_json::to_string(&payload).unwrap());

                continue;
            }
        };

        match engine.inner.compile_with_scope(&scope, &input) {
            Ok(ast) => {
                global_ast += ast;
                let result = engine
                    .inner
                    .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &global_ast);
                global_ast.clear_statements();

                let output = std::mem::take(&mut env.lock().output);
                for entry in output {
                    match entry {
                        ScriptOutputKind::PrintLine(message) => {
                            let payload = serde_json::json! {{
                                "kind": "println",
                                "message": message,
                            }};

                            println!("{}", serde_json::to_string(&payload).unwrap());
                        }
                        ScriptOutputKind::Image { path, data } => {
                            let payload = serde_json::json! {{
                                "kind": "image",
                                "path": path,
                                "data": &data[..]
                            }};

                            println!("{}", serde_json::to_string(&payload).unwrap());
                        }
                    }
                }

                if let Err(error) = result {
                    let p = error.position();
                    let payload = serde_json::json! {{
                        "kind": "runtime_error",
                        "message": error.to_string(),
                        "line": p.line(),
                        "column": p.position()
                    }};

                    println!("{}", serde_json::to_string(&payload).unwrap());
                }
            }
            Err(error) => {
                let p = error.1;
                let payload = serde_json::json! {{
                    "kind": "syntax_error",
                    "message": error.to_string(),
                    "line": p.line(),
                    "column": p.position()
                }};

                println!("{}", serde_json::to_string(&payload).unwrap());
            }
        }

        let payload = serde_json::json! {{
            "kind": "idle"
        }};

        println!("{}", serde_json::to_string(&payload).unwrap());
    }
}

struct ToCodeContext {
    list_source: String,
    output: String,
}

impl AllocationFilter {
    pub fn to_code(&self, list_source: Option<String>) -> String {
        let mut ctx = ToCodeContext {
            list_source: list_source.unwrap_or_else(|| "allocations()".into()),
            output: String::new(),
        };

        self.to_code_impl(&mut ctx);

        ctx.output
    }
}

impl MapFilter {
    pub fn to_code(&self, list_source: Option<String>) -> String {
        let mut ctx = ToCodeContext {
            list_source: list_source.unwrap_or_else(|| "maps()".into()),
            output: String::new(),
        };

        self.to_code_impl(&mut ctx);

        ctx.output
    }
}

trait ToCode {
    fn to_code_impl(&self, ctx: &mut ToCodeContext);
}

impl<T> ToCode for Filter<T>
where
    T: ToCode,
{
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        match *self {
            Filter::Basic(ref filter) => filter.to_code_impl(ctx),
            Filter::And(ref lhs, ref rhs) => {
                write!(&mut ctx.output, "(").unwrap();
                lhs.to_code_impl(ctx);
                write!(&mut ctx.output, " & ").unwrap();
                rhs.to_code_impl(ctx);
                write!(&mut ctx.output, ")").unwrap();
            }
            Filter::Or(ref lhs, ref rhs) => {
                write!(&mut ctx.output, "(").unwrap();
                lhs.to_code_impl(ctx);
                write!(&mut ctx.output, " | ").unwrap();
                rhs.to_code_impl(ctx);
                write!(&mut ctx.output, ")").unwrap();
            }
            Filter::Not(ref filter) => {
                write!(&mut ctx.output, "(!").unwrap();
                filter.to_code_impl(ctx);
                write!(&mut ctx.output, ")").unwrap();
            }
        }
    }
}

impl ToCode for Regex {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        // TODO: Escape the string.
        write!(&mut ctx.output, "\"{}\"", self.as_str()).unwrap();
    }
}

impl ToCode for u32 {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        write!(&mut ctx.output, "{}", self).unwrap();
    }
}

impl ToCode for u64 {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        write!(&mut ctx.output, "{}", self).unwrap();
    }
}

impl ToCode for usize {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        write!(&mut ctx.output, "{}", self).unwrap();
    }
}

impl ToCode for NumberOrFractionOfTotal {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        match *self {
            NumberOrFractionOfTotal::Number(value) => {
                write!(&mut ctx.output, "{}", value).unwrap();
            }
            NumberOrFractionOfTotal::Fraction(value) => {
                write!(&mut ctx.output, "{}", value).unwrap();
            }
        }
    }
}

impl ToCode for Duration {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        if self.0.as_usecs() == 0 {
            write!(&mut ctx.output, "s(0)").unwrap();
            return;
        }

        let mut d = self.decompose();
        d.hours += d.days * 24;

        let mut non_empty = false;
        if d.hours > 0 {
            non_empty = true;
            write!(&mut ctx.output, "h({})", d.hours).unwrap();
        }
        if d.minutes > 0 {
            if non_empty {
                ctx.output.push_str(" + ");
            }
            non_empty = true;
            write!(&mut ctx.output, "m({})", d.minutes).unwrap();
        }
        if d.secs > 0 {
            if non_empty {
                ctx.output.push_str(" + ");
            }
            non_empty = true;
            write!(&mut ctx.output, "s({})", d.secs).unwrap();
        }
        if d.ms > 0 {
            if non_empty {
                ctx.output.push_str(" + ");
            }
            non_empty = true;
            write!(&mut ctx.output, "ms({})", d.ms).unwrap();
        }
        if d.us > 0 {
            if non_empty {
                ctx.output.push_str(" + ");
            }
            write!(&mut ctx.output, "us({})", d.us).unwrap();
        }
    }
}

impl ToCode for HashSet<BacktraceId> {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        ctx.output.push_str("[");
        let mut is_first = true;
        for item in self {
            if is_first {
                is_first = false;
            } else {
                ctx.output.push_str(", ");
            }
            write!(&mut ctx.output, "{}", item.raw()).unwrap();
        }
        ctx.output.push_str("]");
    }
}

impl ToCode for HashSet<MapId> {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        ctx.output.push_str("[");
        let mut is_first = true;
        for item in self {
            if is_first {
                is_first = false;
            } else {
                ctx.output.push_str(", ");
            }
            write!(&mut ctx.output, "{}", item.raw()).unwrap();
        }
        ctx.output.push_str("]");
    }
}

macro_rules! out {
    ($ctx:expr => $($field:ident.$name:ident)+) => {
        $(
            if let Some( ref value ) = $field.$name {
                write!( &mut $ctx.output, "  .{}(", stringify!( $name ) ).unwrap();
                value.to_code_impl( $ctx );
                writeln!( &mut $ctx.output, ")" ).unwrap();
            }
        )+
    };
}

macro_rules! out_vec_if_not_empty {
    ($ctx:expr => $($field:ident.$name:ident)+) => {
        $(
            if !$field.$name.is_empty() {
                write!( &mut $ctx.output, "  .{}([", stringify!( $name ) ).unwrap();
                let mut is_first = true;
                for value in &$field.$name {
                    if is_first {
                        is_first = false;
                    } else {
                        write!( &mut $ctx.output, ", " ).unwrap();
                    }
                    value.to_code_impl( $ctx );
                }

                writeln!( &mut $ctx.output, "])" ).unwrap();
            }
        )+
    };
}

macro_rules! out_bool {
    ($ctx:expr => $($field:ident.$name:ident)+) => {
        $(
            if $field.$name {
                writeln!( &mut $ctx.output, "  .{}()", stringify!( $name ) ).unwrap();
            }
        )+
    }
}

impl ToCode for crate::filter::RawBacktraceFilter {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        out! { ctx =>
            self.only_passing_through_function
            self.only_not_passing_through_function
            self.only_passing_through_source
            self.only_not_passing_through_source
            self.only_matching_backtraces
            self.only_not_matching_backtraces
            self.only_backtrace_length_at_least
            self.only_backtrace_length_at_most
        }
    }
}

impl ToCode for crate::filter::RawCommonFilter {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        out! { ctx =>
            self.only_larger_or_equal
            self.only_larger
            self.only_smaller_or_equal
            self.only_smaller
            self.only_address_at_least
            self.only_address_at_most
            self.only_allocated_after_at_least
            self.only_allocated_until_at_most
            self.only_deallocated_after_at_least
            self.only_deallocated_until_at_most
            self.only_alive_for_at_least
            self.only_alive_for_at_most
            self.only_leaked_or_deallocated_after
        }

        out_vec_if_not_empty! { ctx =>
            self.only_alive_at
        }

        out_bool! { ctx =>
            self.only_leaked
            self.only_temporary
        }
    }
}

impl ToCode for RawAllocationFilter {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        writeln!(&mut ctx.output, "{}", ctx.list_source).unwrap();

        self.backtrace_filter.to_code_impl(ctx);
        self.common_filter.to_code_impl(ctx);

        out! { ctx =>
            self.only_first_size_larger_or_equal
            self.only_first_size_larger
            self.only_first_size_smaller_or_equal
            self.only_first_size_smaller
            self.only_last_size_larger_or_equal
            self.only_last_size_larger
            self.only_last_size_smaller_or_equal
            self.only_last_size_smaller
            self.only_chain_length_at_least
            self.only_chain_length_at_most
            self.only_chain_alive_for_at_least
            self.only_chain_alive_for_at_most
            self.only_position_in_chain_at_least
            self.only_position_in_chain_at_most

            self.only_group_allocations_at_least
            self.only_group_allocations_at_most
            self.only_group_interval_at_least
            self.only_group_interval_at_most
            self.only_group_max_total_usage_first_seen_at_least
            self.only_group_max_total_usage_first_seen_at_most
            self.only_group_leaked_allocations_at_least
            self.only_group_leaked_allocations_at_most

            self.only_with_marker

            self.only_from_maps
        }

        out_bool! { ctx =>
            self.only_chain_leaked
            self.only_ptmalloc_mmaped
            self.only_ptmalloc_not_mmaped
            self.only_ptmalloc_from_main_arena
            self.only_ptmalloc_not_from_main_arena
            self.only_jemalloc
            self.only_not_jemalloc
        }
    }
}

impl ToCode for RawMapFilter {
    fn to_code_impl(&self, ctx: &mut ToCodeContext) {
        writeln!(&mut ctx.output, "{}", ctx.list_source).unwrap();

        self.backtrace_filter.to_code_impl(ctx);
        self.common_filter.to_code_impl(ctx);

        out! { ctx =>
            self.only_peak_rss_at_least
            self.only_peak_rss_at_most
        }

        out_bool! { ctx =>
            self.only_jemalloc
            self.only_not_jemalloc
            self.only_bytehound
            self.only_not_bytehound
            self.only_readable
            self.only_not_readable
            self.only_writable
            self.only_not_writable
            self.only_executable
            self.only_not_executable
        }
    }
}
