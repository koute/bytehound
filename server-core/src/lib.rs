#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

use actix_web::{
    body::{BodyStream, BoxBody},
    web, App, HttpRequest, HttpResponse, Result,
};

use std::borrow::Cow;
use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Write};
use std::fs::File;
use std::io;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use ahash::AHashMap as HashMap;

use actix_cors::Cors;
use actix_web::error::Error as ActixWebError;
use actix_web::error::{ErrorBadRequest, ErrorNotFound};

use itertools::Itertools;
use lru::LruCache;
use parking_lot::Mutex;
use rayon::prelude::*;
use serde::Serialize;

use cli_core::{
    export_as_flamegraph, export_as_flamegraph_pl, export_as_heaptrack, export_as_replay,
    table_to_string, Allocation, AllocationId, BacktraceId, CountAndSize, Data, DataId, Frame,
    FrameId, Loader, MalloptKind, Map, MapId, NodeId, OperationId, RegionFlags, Tree, VecVec,
};

use common::Timestamp;

mod byte_channel;
mod filter;
mod itertools;
mod protocol;
mod streaming_channel;
mod streaming_serializer;

use crate::byte_channel::byte_channel;
use crate::filter::{
    prepare_allocation_filter, prepare_map_filter, prepare_raw_allocation_filter,
    prepare_raw_map_filter, AllocationFilter, PrepareFilterError,
};
use crate::streaming_serializer::StreamingSerializer;

struct AllocationGroups {
    allocations_by_backtrace: VecVec<BacktraceId, AllocationId>,
}

impl AllocationGroups {
    fn new<'a, T: ParallelIterator<Item = (AllocationId, &'a Allocation)>>(iter: T) -> Self {
        let grouped = iter
            .fold_with(HashMap::new(), |mut grouped, (id, allocation)| {
                grouped
                    .entry(allocation.backtrace)
                    .or_insert_with(Vec::new)
                    .push(id);
                grouped
            })
            .reduce(
                || HashMap::new(),
                |mut a, mut b| {
                    if b.len() > a.len() {
                        std::mem::swap(&mut a, &mut b);
                    }

                    for (backtrace, ids) in b {
                        a.entry(backtrace).or_insert_with(Vec::new).extend(ids);
                    }

                    a
                },
            );

        let mut grouped: Vec<(BacktraceId, Vec<AllocationId>)> = grouped.into_iter().collect();
        grouped.par_sort_by_key(|&(backtrace_id, _)| backtrace_id);

        let mut allocations = VecVec::new();
        for (backtrace_id, allocation_ids) in grouped {
            allocations.insert(backtrace_id, allocation_ids);
        }

        let groups = AllocationGroups {
            allocations_by_backtrace: allocations,
        };

        groups
    }

    fn len(&self) -> usize {
        self.allocations_by_backtrace.len()
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AllocationGroupsKey {
    data_id: DataId,
    filter: protocol::AllocFilter,
    custom_filter: protocol::CustomFilter,
    sort_by: protocol::AllocGroupsSortBy,
    order: protocol::Order,
}

#[derive(Clone)]
struct GeneratedFile {
    timestamp: Instant,
    hash: String,
    mime: &'static str,
    data: Arc<Vec<u8>>,
}

#[derive(Default)]
struct GeneratedFilesCollection {
    by_hash: HashMap<String, GeneratedFile>,
    total_size: usize,
}

impl GeneratedFilesCollection {
    fn purge_old_if_too_big(&mut self) {
        if self.total_size < 32 * 1024 * 1024 {
            return;
        }

        let mut list: Vec<_> = self.by_hash.values().cloned().collect();
        list.sort_by_key(|entry| entry.timestamp);
        list.reverse();

        while let Some(entry) = list.pop() {
            if self.total_size <= 16 * 1024 * 1024 {
                break;
            }

            self.total_size -= entry.data.len();
            self.by_hash.remove(&entry.hash);
        }
    }

    fn add_file(&mut self, entry: GeneratedFile) {
        if !self.by_hash.contains_key(&entry.hash) {
            self.total_size += entry.data.len();
            self.by_hash.insert(entry.hash.clone(), entry);
        }
    }
}

struct State {
    data: HashMap<DataId, Arc<Data>>,
    data_ids: Vec<DataId>,
    allocation_group_cache: Mutex<LruCache<AllocationGroupsKey, Arc<AllocationGroups>>>,
    generated_files: Mutex<GeneratedFilesCollection>,
}

impl State {
    fn new() -> Self {
        State {
            data: HashMap::new(),
            data_ids: Vec::new(),
            allocation_group_cache: Mutex::new(LruCache::new(NonZeroUsize::new(4).unwrap())),
            generated_files: Default::default(),
        }
    }

    fn add_data(&mut self, data: Data) {
        if self.data.contains_key(&data.id()) {
            return;
        }

        self.data_ids.push(data.id());
        self.data.insert(data.id(), Arc::new(data));
    }

    fn last_id(&self) -> Option<DataId> {
        self.data_ids.last().cloned()
    }

    fn generate_graphs(
        &self,
        data: &Data,
        args: cli_core::script::EngineArgs,
        code: &str,
    ) -> Vec<String> {
        let env = Arc::new(Mutex::new(cli_core::script::VirtualEnvironment::new()));
        let engine = cli_core::script::Engine::new(env.clone(), args);
        engine.run(&code).unwrap();
        let mut urls = Vec::new();
        let files = std::mem::take(&mut env.lock().output);
        for file in files {
            match file {
                cli_core::script::ScriptOutputKind::Image { path, data: bytes } => {
                    let hash = format!("{:x}", md5::compute(&*bytes));
                    let basename = path[path.rfind("/").unwrap() + 1..].to_owned();
                    let url = format!("/data/{}/script_files/{}/{}", data.id(), hash, basename);
                    let entry = GeneratedFile {
                        timestamp: Instant::now(),
                        hash,
                        mime: "image/svg+xml",
                        data: bytes,
                    };

                    let mut generated = self.generated_files.lock();
                    generated.purge_old_if_too_big();
                    generated.add_file(entry);

                    urls.push(url);
                }
                _ => {}
            }
        }

        urls
    }
}

type StateRef = Arc<State>;

trait StateGetter {
    fn state(&self) -> &StateRef;
}

impl StateGetter for HttpRequest {
    fn state(&self) -> &StateRef {
        self.app_data::<web::Data<StateRef>>().unwrap()
    }
}

fn query<'a, T: serde::Deserialize<'a>>(req: &'a HttpRequest) -> Result<T> {
    serde_urlencoded::from_str::<T>(req.query_string()).map_err(|e| e.into())
}

fn get_data_id(req: &HttpRequest) -> Result<DataId> {
    let id = req.match_info().get("id").unwrap();
    if id == "last" {
        return req.state().last_id().ok_or(ErrorNotFound("data not found"));
    }

    let id: DataId = id.parse().map_err(|_| ErrorNotFound("data not found"))?;
    if !req.state().data.contains_key(&id) {
        return Err(ErrorNotFound("data not found"));
    }
    Ok(id)
}

fn get_data(req: &HttpRequest) -> Result<&Arc<Data>> {
    let id = get_data_id(req)?;
    req.state()
        .data
        .get(&id)
        .ok_or_else(|| ErrorNotFound("data not found"))
}

impl From<PrepareFilterError> for ActixWebError {
    fn from(error: PrepareFilterError) -> Self {
        match error {
            PrepareFilterError::InvalidRegex(field, inner_err) => {
                ErrorBadRequest(format!("invalid '{}': {}", field, inner_err))
            }
            PrepareFilterError::InvalidCustomFilter(message) => {
                ErrorBadRequest(format!("failed to evaluate custom filter: {}", message))
            }
        }
    }
}

fn async_data_handler<F: FnOnce(Arc<Data>, byte_channel::ByteSender) + Send + 'static>(
    req: &HttpRequest,
    callback: F,
) -> Result<BoxBody> {
    let (tx, rx) = byte_channel();
    // let rx = rx.map_err(|_| ErrorInternalServerError("internal error"));
    let rx = BodyStream::new(rx);
    let body = BoxBody::new(rx);

    let data_id = get_data_id(&req)?;
    let state = req.state().clone();
    thread::spawn(move || {
        let data = match state.data.get(&data_id) {
            Some(data) => data,
            None => return,
        };

        callback(data.clone(), tx);
    });

    Ok(body)
}

fn strip_template(input: &str) -> String {
    let mut out = String::new();
    let mut buffered = String::new();
    let mut depth = 0;
    for ch in input.chars() {
        if ch == '<' {
            if out.ends_with("operator") {
                out.push(ch);
                continue;
            }

            if depth == 0 {
                buffered.clear();
                out.push(ch);
            } else {
                buffered.push(ch);
            }

            depth += 1;
            continue;
        }

        if depth > 0 {
            if ch == '>' {
                depth -= 1;
                if depth == 0 {
                    out.push_str("...");
                    out.push(ch);
                    buffered.clear();
                }

                continue;
            }
            buffered.push(ch);
            continue;
        }

        out.push(ch);
    }

    out.push_str(&buffered);
    out
}

fn get_frame<'a>(
    data: &'a Data,
    format: &protocol::BacktraceFormat,
    frame: &Frame,
) -> protocol::Frame<'a> {
    let mut function = frame
        .function()
        .map(|id| Cow::Borrowed(data.interner().resolve(id).unwrap()));
    if format.strip_template_args.unwrap_or(false) {
        function = function.map(|function| strip_template(&function).into());
    }

    protocol::Frame {
        address: frame.address().raw(),
        address_s: format!("{:016X}", frame.address().raw()),
        count: frame.count(),
        library: frame
            .library()
            .map(|id| data.interner().resolve(id).unwrap()),
        function,
        raw_function: frame
            .raw_function()
            .map(|id| data.interner().resolve(id).unwrap()),
        source: frame
            .source()
            .map(|id| data.interner().resolve(id).unwrap()),
        line: frame.line(),
        column: frame.column(),
        is_inline: frame.is_inline(),
    }
}

impl protocol::ResponseMetadata {
    fn new(data: &Data) -> Self {
        protocol::ResponseMetadata {
            id: format!("{}", data.id()),
            executable: data.executable().to_owned(),
            cmdline: data
                .cmdline()
                .into_iter()
                .map(|arg| {
                    if arg.contains(" ") {
                        format!("\"{}\"", arg)
                    } else {
                        arg
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            architecture: data.architecture().to_owned(),
            final_allocated: data.total_allocated() - data.total_freed(),
            final_allocated_count: data.total_allocated_count() - data.total_freed_count(),
            runtime: (data.last_timestamp() - data.initial_timestamp()).into(),
            unique_backtrace_count: data.unique_backtrace_count() as u64,
            maximum_backtrace_depth: data.maximum_backtrace_depth(),
            timestamp: data.initial_timestamp().into(),
        }
    }
}

async fn handler_list(req: HttpRequest) -> HttpResponse {
    let list: Vec<_> = req
        .state()
        .data
        .values()
        .map(|data| protocol::ResponseMetadata::new(data))
        .collect();

    HttpResponse::Ok().json(list)
}

fn build_timeline(data: &Data, ops: &[OperationId]) -> protocol::ResponseTimeline {
    let timeline = cli_core::build_allocation_timeline(
        data,
        data.initial_timestamp(),
        data.last_timestamp(),
        ops,
    );

    let mut xs = Vec::with_capacity(timeline.len());
    let mut size_delta = Vec::with_capacity(timeline.len());
    let mut count_delta = Vec::with_capacity(timeline.len());
    let mut allocated_size = Vec::with_capacity(timeline.len());
    let mut allocated_count = Vec::with_capacity(timeline.len());
    let mut allocations = Vec::with_capacity(timeline.len());
    let mut deallocations = Vec::with_capacity(timeline.len());

    let mut last_size = 0;
    let mut last_count = 0;
    for point in timeline {
        xs.push(point.timestamp / 1000);
        size_delta.push(point.memory_usage as i64 - last_size);
        count_delta.push(point.allocations as i64 - last_count);
        allocated_size.push(point.memory_usage as u64);
        allocated_count.push(point.allocations as u64);
        allocations.push(point.positive_change.allocations as u32);
        deallocations.push(point.negative_change.allocations as u32);

        last_size = point.memory_usage as i64;
        last_count = point.allocations as i64;
    }

    protocol::ResponseTimeline {
        xs,
        size_delta,
        count_delta,
        allocated_size,
        allocated_count,
        allocations,
        deallocations,
    }
}

async fn handler_timeline(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let timeline = build_timeline(&data, data.operation_ids());
    Ok(HttpResponse::Ok().json(timeline))
}

async fn handler_timeline_leaked(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let ops: Vec<_> = data
        .operation_ids()
        .par_iter()
        .flat_map(|op| {
            let allocation = data.get_allocation(op.id());
            if allocation.deallocation.is_some() {
                None
            } else {
                Some(OperationId::new_allocation(op.id()))
            }
        })
        .collect();

    let timeline = build_timeline(&data, &ops);
    Ok(HttpResponse::Ok().json(timeline))
}

async fn handler_timeline_maps(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let mut ops = Vec::new();
    for map in data.maps() {
        if map.is_from_bytehound() {
            continue;
        }

        map.emit_ops(&mut ops);
    }

    ops.par_sort_by_key(|(timestamp, _)| *timestamp);

    let timestamp_min = ops
        .first()
        .map(|(timestamp, _)| *timestamp)
        .unwrap_or(common::Timestamp::min());
    let timestamp_max = ops
        .last()
        .map(|(timestamp, _)| *timestamp)
        .unwrap_or(common::Timestamp::min());

    let timeline = cli_core::build_map_timeline(timestamp_min, timestamp_max, &ops);
    std::mem::drop(ops);

    let mut xs = Vec::with_capacity(timeline.len());
    let mut address_space = Vec::with_capacity(timeline.len());
    let mut rss = Vec::with_capacity(timeline.len());
    let mut anonymous = Vec::with_capacity(timeline.len());
    let mut dirty = Vec::with_capacity(timeline.len());
    let mut clean = Vec::with_capacity(timeline.len());
    let mut swap = Vec::with_capacity(timeline.len());

    for point in timeline {
        xs.push(point.timestamp / 1000);
        address_space.push(point.address_space as i64);
        rss.push(point.rss() as i64);
        anonymous.push(point.anonymous as i64);
        dirty.push(point.dirty() as i64);
        clean.push(point.clean() as i64);
        swap.push(point.swap as i64);
    }

    let timeline = protocol::ResponseMapTimeline {
        xs,
        address_space,
        rss,
        anonymous,
        dirty,
        clean,
        swap,
    };

    Ok(HttpResponse::Ok().json(timeline))
}

fn prefiltered_allocation_ids<'a>(
    data: &'a Data,
    sort_by: protocol::AllocSortBy,
    _filter: &AllocationFilter,
) -> &'a [AllocationId] {
    // TODO: Use the filter to narrow down the range.
    match sort_by {
        protocol::AllocSortBy::Timestamp => data.alloc_sorted_by_timestamp(None, None),
        protocol::AllocSortBy::Address => data.alloc_sorted_by_address(None, None),
        protocol::AllocSortBy::Size => data.alloc_sorted_by_size(None, None),
    }
}

fn allocations_iter<'a>(
    data: &'a Data,
    array: &'a [AllocationId],
    order: protocol::Order,
    filter: AllocationFilter,
) -> Box<dyn DoubleEndedIterator<Item = (AllocationId, &'a Allocation)> + 'a> {
    match order {
        protocol::Order::Asc => Box::new(
            array
                .iter()
                .map(move |&id| (id, data.get_allocation(id)))
                .filter(move |(id, allocation)| filter.try_match(data, *id, allocation)),
        ),
        protocol::Order::Dsc => Box::new(
            array
                .iter()
                .rev()
                .map(move |&id| (id, data.get_allocation(id)))
                .filter(move |(id, allocation)| filter.try_match(data, *id, allocation)),
        ),
    }
}

fn timestamp_to_fraction(data: &Data, timestamp: Timestamp) -> f32 {
    let relative = timestamp - data.initial_timestamp();
    let range = data.last_timestamp() - data.initial_timestamp();
    (relative.as_usecs() as f64 / range.as_usecs() as f64) as f32
}

fn get_allocations<'a>(
    data: &'a Arc<Data>,
    backtrace_format: protocol::BacktraceFormat,
    params: protocol::RequestAllocations,
    filter: crate::filter::AllocationFilter,
) -> protocol::ResponseAllocations<impl Serialize + 'a> {
    let remaining = params.count.unwrap_or(-1_i32 as _) as usize;
    let skip = params.skip.unwrap_or(0) as usize;
    let sort_by = params.sort_by.unwrap_or(protocol::AllocSortBy::Timestamp);
    let order = params.order.unwrap_or(protocol::Order::Asc);

    let allocation_ids = prefiltered_allocation_ids(data, sort_by, &filter);
    let total_count = allocation_ids
        .par_iter()
        .filter(|&&id| filter.try_match(data, id, data.get_allocation(id)))
        .count() as u64;

    let allocations = move || {
        let backtrace_format = backtrace_format.clone();
        let filter = filter.clone();

        allocations_iter(data, allocation_ids, order, filter)
            .skip(skip)
            .take(remaining)
            .map(move |(allocation_id, allocation)| {
                let backtrace = data
                    .get_backtrace(allocation.backtrace)
                    .map(|(_, frame)| get_frame(data, &backtrace_format, frame))
                    .collect();
                let chain = data.get_chain_by_any_allocation(allocation_id);
                protocol::Allocation {
                    id: allocation_id.raw(),
                    address: allocation.pointer,
                    address_s: format!("{:016X}", allocation.pointer),
                    timestamp: allocation.timestamp.into(),
                    timestamp_relative: (allocation.timestamp - data.initial_timestamp()).into(),
                    timestamp_relative_p: timestamp_to_fraction(data, allocation.timestamp),
                    thread: allocation.thread,
                    size: allocation.size,
                    backtrace_id: allocation.backtrace.raw(),
                    deallocation: allocation.deallocation.as_ref().map(|deallocation| {
                        protocol::Deallocation {
                            timestamp: deallocation.timestamp.into(),
                            thread: deallocation.thread,
                            backtrace_id: deallocation.backtrace.map(|id| id.raw()),
                            backtrace: deallocation.backtrace.map(|backtrace_id| {
                                data.get_backtrace(backtrace_id)
                                    .map(|(_, frame)| get_frame(data, &backtrace_format, frame))
                                    .collect()
                            }),
                        }
                    }),
                    chain_deallocation: if chain.last == allocation_id {
                        None
                    } else {
                        data.get_allocation(chain.last)
                            .deallocation
                            .as_ref()
                            .map(|deallocation| protocol::Deallocation {
                                timestamp: deallocation.timestamp.into(),
                                thread: deallocation.thread,
                                backtrace_id: deallocation.backtrace.map(|id| id.raw()),
                                backtrace: deallocation.backtrace.map(|backtrace_id| {
                                    data.get_backtrace(backtrace_id)
                                        .map(|(_, frame)| get_frame(data, &backtrace_format, frame))
                                        .collect()
                                }),
                            })
                    },
                    backtrace,
                    in_main_arena: !allocation.in_non_main_arena(),
                    is_mmaped: allocation.is_mmaped(),
                    is_jemalloc: allocation.is_jemalloc(),
                    extra_space: allocation.extra_usable_space,
                    chain_lifetime: chain.lifetime(data).map(|lifetime| lifetime.into()),
                    position_in_chain: allocation.position_in_chain,
                    chain_length: chain.length,
                }
            })
    };

    protocol::ResponseAllocations {
        allocations: StreamingSerializer::new(allocations),
        total_count,
    }
}

async fn handler_allocations(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let params: protocol::RequestAllocations = query(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter, &custom_filter)?;
    let backtrace_format: protocol::BacktraceFormat = query(&req)?;

    let body = async_data_handler(&req, move |data, tx| {
        let response = get_allocations(&data, backtrace_format, params, filter);
        let _ = serde_json::to_writer(tx, &response);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(body))
}

fn get_regions<'a>(
    data: &'a Data,
    backtrace_format: &protocol::BacktraceFormat,
    map: &'a Map,
) -> Vec<protocol::MapRegion<'a>> {
    map.regions
        .iter()
        .map(|region| protocol::MapRegion {
            address: region.pointer,
            address_s: format!("{:016X}", region.pointer),
            timestamp: region.timestamp.into(),
            timestamp_relative: (region.timestamp - data.initial_timestamp()).into(),
            timestamp_relative_p: timestamp_to_fraction(data, region.timestamp),
            size: region.size,
            deallocation: region.deallocation.as_ref().map(|deallocation| {
                protocol::MapRegionDeallocation {
                    timestamp: deallocation.timestamp.into(),
                    sources: deallocation
                        .sources
                        .iter()
                        .map(|source| protocol::MapRegionDeallocationSource {
                            address: source.address,
                            length: source.length,
                            source: protocol::MapSource {
                                timestamp: source.source.timestamp.into(),
                                timestamp_relative: (source.source.timestamp
                                    - data.initial_timestamp())
                                .into(),
                                timestamp_relative_p: timestamp_to_fraction(
                                    data,
                                    source.source.timestamp,
                                ),
                                thread: source.source.thread,
                                backtrace_id: source.source.backtrace.raw(),
                                backtrace: data
                                    .get_backtrace(source.source.backtrace)
                                    .map(|(_, frame)| get_frame(data, backtrace_format, frame))
                                    .collect(),
                            },
                        })
                        .collect(),
                }
            }),
            file_offset: region.file_offset,
            inode: region.inode,
            major: region.major,
            minor: region.minor,
            name: (&*region.name).into(),
            is_readable: region.flags.contains(RegionFlags::READABLE),
            is_writable: region.flags.contains(RegionFlags::WRITABLE),
            is_executable: region.flags.contains(RegionFlags::EXECUTABLE),
            is_shared: region.flags.contains(RegionFlags::SHARED),
        })
        .collect()
}

fn get_maps<'a>(
    state: Arc<State>,
    data: &'a Arc<Data>,
    backtrace_format: protocol::BacktraceFormat,
    params: protocol::RequestMaps,
    filter: crate::filter::MapFilter,
) -> protocol::ResponseMaps<impl Serialize + 'a> {
    let remaining = params.count.unwrap_or(-1_i32 as _) as usize;
    let skip = params.skip.unwrap_or(0) as usize;
    let sort_by = params.sort_by.unwrap_or(protocol::MapsSortBy::Timestamp);
    let order = params.order.unwrap_or(protocol::Order::Asc);
    let generate_graphs = params.generate_graphs.unwrap_or(false);

    let mut list = Vec::new();
    if let Some(id) = params.id {
        let id = MapId(id as u64);
        if data.contains_map(id) {
            list.push(id);
        }
    } else {
        list = data
            .maps()
            .par_iter()
            .enumerate()
            .map(|(index, map)| {
                let id = MapId(index as u64);
                (id, map)
            })
            .filter(|(id, map)| filter.try_match(data, *id, map))
            .map(|(id, _)| id)
            .collect();
    }

    let cmp = match sort_by {
        protocol::MapsSortBy::Timestamp => {
            if order == protocol::Order::Asc {
                |a: &Map, b: &Map| a.timestamp.cmp(&b.timestamp)
            } else {
                |a: &Map, b: &Map| b.timestamp.cmp(&a.timestamp)
            }
        }
        protocol::MapsSortBy::Address => {
            if order == protocol::Order::Asc {
                |a: &Map, b: &Map| a.pointer.cmp(&b.pointer)
            } else {
                |a: &Map, b: &Map| b.pointer.cmp(&a.pointer)
            }
        }
        protocol::MapsSortBy::Size => {
            if order == protocol::Order::Asc {
                |a: &Map, b: &Map| a.size.cmp(&b.size)
            } else {
                |a: &Map, b: &Map| b.size.cmp(&a.size)
            }
        }
        protocol::MapsSortBy::PeakRss => {
            if order == protocol::Order::Asc {
                |a: &Map, b: &Map| a.peak_rss.cmp(&b.peak_rss)
            } else {
                |a: &Map, b: &Map| b.peak_rss.cmp(&a.peak_rss)
            }
        }
    };

    list.par_sort_by(move |a, b| {
        let a = &data.maps()[a.0 as usize];
        let b = &data.maps()[b.0 as usize];
        cmp(a, b)
    });

    let total_count = list.len() as u64;

    let maps = move || {
        let backtrace_format = backtrace_format.clone();

        list.into_iter().skip(skip).take(remaining).map(move |id| {
            let map = &data.maps()[id.0 as usize];
            let source = map.source.map(|source| protocol::MapSource {
                timestamp: source.timestamp.into(),
                timestamp_relative: (source.timestamp - data.initial_timestamp()).into(),
                timestamp_relative_p: timestamp_to_fraction(data, source.timestamp),
                thread: source.thread,
                backtrace_id: source.backtrace.raw(),
                backtrace: data
                    .get_backtrace(source.backtrace)
                    .map(|(_, frame)| get_frame(data, &backtrace_format, frame))
                    .collect(),
            });

            let mut graph_preview_url = None;
            let mut graph_url = None;

            if generate_graphs {
                let code = format!(
                    r#"
                        let graph = graph()
                            .add(maps())
                            .save()
                            .without_axes()
                            .without_legend()
                            .save();
                    "#
                );

                let args = cli_core::script::EngineArgs {
                    data: Some(data.clone()),
                    map_ids: Some(Arc::new(vec![id])),
                    ..cli_core::script::EngineArgs::default()
                };

                let mut urls = state.generate_graphs(&data, args, &code).into_iter();
                graph_url = urls.next();
                graph_preview_url = urls.next();
            }

            let regions = if !params.with_regions.unwrap_or(false) {
                None
            } else {
                Some(get_regions(&data, &backtrace_format, map))
            };

            let usage_history = if !params.with_usage_history.unwrap_or(false) {
                None
            } else {
                Some(
                    map.usage_history
                        .iter()
                        .map(|usage| protocol::MapUsage {
                            timestamp: usage.timestamp.into(),
                            timestamp_relative: (usage.timestamp - data.initial_timestamp()).into(),
                            timestamp_relative_p: timestamp_to_fraction(data, usage.timestamp),
                            address_space: usage.address_space,
                            anonymous: usage.anonymous,
                            shared_clean: usage.shared_clean,
                            shared_dirty: usage.shared_dirty,
                            private_clean: usage.private_clean,
                            private_dirty: usage.private_dirty,
                            swap: usage.swap,
                            rss: usage.rss(),
                        })
                        .collect(),
                )
            };

            protocol::Map {
                id: id.0,

                address: map.pointer,
                address_s: format!("{:016X}", map.pointer),
                timestamp: map.timestamp.into(),
                timestamp_relative: (map.timestamp - data.initial_timestamp()).into(),
                timestamp_relative_p: timestamp_to_fraction(data, map.timestamp),
                size: map.size,
                source,
                deallocation: map.deallocation.as_ref().map(|deallocation| {
                    protocol::MapDeallocation {
                        timestamp: deallocation.timestamp.into(),
                        source: deallocation.source.map(|source| protocol::MapSource {
                            timestamp: source.timestamp.into(),
                            timestamp_relative: (source.timestamp - data.initial_timestamp())
                                .into(),
                            timestamp_relative_p: timestamp_to_fraction(data, source.timestamp),
                            thread: source.thread,
                            backtrace_id: source.backtrace.raw(),
                            backtrace: data
                                .get_backtrace(source.backtrace)
                                .map(|(_, frame)| get_frame(data, &backtrace_format, frame))
                                .collect(),
                        }),
                    }
                }),
                name: (&*map.name).into(),
                is_readable: map.flags.contains(RegionFlags::READABLE),
                is_writable: map.flags.contains(RegionFlags::WRITABLE),
                is_executable: map.flags.contains(RegionFlags::EXECUTABLE),
                is_shared: map.flags.contains(RegionFlags::SHARED),

                peak_rss: map.peak_rss,
                graph_preview_url,
                graph_url,
                regions,
                usage_history,
            }
        })
    };

    protocol::ResponseMaps {
        maps: StreamingSerializer::new(maps),
        total_count,
    }
}

async fn handler_maps(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let params: protocol::RequestMaps = query(&req)?;
    let filter: protocol::MapFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_map_filter(data, &filter, &custom_filter)?;
    let backtrace_format: protocol::BacktraceFormat = query(&req)?;
    let state = req.state().clone();

    let body = async_data_handler(&req, move |data, tx| {
        let response = get_maps(state, &data, backtrace_format, params, filter);
        let _ = serde_json::to_writer(tx, &response);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(body))
}

fn get_allocation_group_data<'a, I>(data: &Data, iter: I) -> protocol::AllocationGroupData
where
    I: ParallelIterator<Item = &'a Allocation>,
{
    #[derive(Clone)]
    struct Group {
        size_sum: u64,
        min_size: u64,
        max_size: u64,
        min_timestamp: Timestamp,
        max_timestamp: Timestamp,
        leaked_count: u64,
        allocated_count: u64,
    }

    impl Default for Group {
        fn default() -> Self {
            Group {
                size_sum: 0,
                min_size: !0,
                max_size: 0,
                min_timestamp: Timestamp::max(),
                max_timestamp: Timestamp::min(),
                leaked_count: 0,
                allocated_count: 0,
            }
        }
    }

    let group = iter
        .fold_with(Group::default(), |mut group, allocation| {
            let size = allocation.size;
            let timestamp = allocation.timestamp;
            group.size_sum += size;
            group.min_size = min(group.min_size, size);
            group.max_size = max(group.max_size, size);
            group.min_timestamp = min(group.min_timestamp, timestamp);
            group.max_timestamp = max(group.max_timestamp, timestamp);

            group.allocated_count += 1;
            if allocation.deallocation.is_none() {
                group.leaked_count += 1;
            }

            group
        })
        .reduce(
            || Group::default(),
            |mut a, b| {
                a.size_sum += b.size_sum;
                a.min_size = min(a.min_size, b.min_size);
                a.max_size = max(a.max_size, b.max_size);
                a.min_timestamp = min(a.min_timestamp, b.min_timestamp);
                a.max_timestamp = max(a.max_timestamp, b.max_timestamp);
                a.allocated_count += b.allocated_count;
                a.leaked_count += b.leaked_count;

                a
            },
        );

    protocol::AllocationGroupData {
        leaked_count: group.leaked_count,
        allocated_count: group.allocated_count,
        size: group.size_sum,
        min_size: group.min_size,
        max_size: group.max_size,
        min_timestamp: group.min_timestamp.into(),
        min_timestamp_relative: (group.min_timestamp - data.initial_timestamp()).into(),
        min_timestamp_relative_p: timestamp_to_fraction(data, group.min_timestamp),
        max_timestamp: group.max_timestamp.into(),
        max_timestamp_relative: (group.max_timestamp - data.initial_timestamp()).into(),
        max_timestamp_relative_p: timestamp_to_fraction(data, group.max_timestamp),
        interval: (group.max_timestamp - group.min_timestamp).into(),
        graph_url: None,
        graph_preview_url: None,
        max_total_usage_first_seen_at: None,
        max_total_usage_first_seen_at_relative: None,
        max_total_usage_first_seen_at_relative_p: None,
    }
}

fn get_global_group_data(data: &Data, backtrace_id: BacktraceId) -> protocol::AllocationGroupData {
    let stats = data.get_group_statistics(backtrace_id);

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
        min_timestamp_relative_p: timestamp_to_fraction(data, min_timestamp),
        max_timestamp: max_timestamp.into(),
        max_timestamp_relative: (max_timestamp - data.initial_timestamp()).into(),
        max_timestamp_relative_p: timestamp_to_fraction(data, max_timestamp),
        interval: (max_timestamp - min_timestamp).into(),
        graph_url: None,
        graph_preview_url: None,
        max_total_usage_first_seen_at: Some(stats.max_total_usage_first_seen_at.into()),
        max_total_usage_first_seen_at_relative: Some(
            (stats.max_total_usage_first_seen_at - data.initial_timestamp()).into(),
        ),
        max_total_usage_first_seen_at_relative_p: Some(timestamp_to_fraction(
            data,
            stats.max_total_usage_first_seen_at,
        )),
    }
}

fn get_allocation_groups<'a>(
    state: &'a State,
    data: &'a Arc<Data>,
    backtrace_format: protocol::BacktraceFormat,
    params: protocol::RequestAllocationGroups,
    allocation_groups: Arc<AllocationGroups>,
) -> protocol::ResponseAllocationGroups<impl Serialize + 'a> {
    let remaining = params.count.unwrap_or(-1_i32 as _) as usize;
    let skip = params.skip.unwrap_or(0) as usize;
    let generate_graphs = params.generate_graphs.unwrap_or(false);

    let total_count = allocation_groups.len();
    let factory = move || {
        let backtrace_format = backtrace_format.clone();
        let allocations = allocation_groups.clone();
        (0..allocations.allocations_by_backtrace.len())
            .skip(skip)
            .take(remaining)
            .map(move |index| {
                let (&backtrace_id, matched_allocation_ids) =
                    allocations.allocations_by_backtrace.get(index);
                let all = get_global_group_data(data, backtrace_id);
                let mut only_matched = get_allocation_group_data(
                    data,
                    matched_allocation_ids
                        .into_par_iter()
                        .map(|&allocation_id| data.get_allocation(allocation_id)),
                );
                let backtrace = data
                    .get_backtrace(backtrace_id)
                    .map(|(_, frame)| get_frame(data, &backtrace_format, frame))
                    .collect();

                if generate_graphs {
                    let code = format!(
                        r#"
                        let graph = graph()
                            .add("Matched", allocations())
                            .add("Global", data().allocations().only_matching_backtraces([{}]))
                            .save()
                            .without_axes()
                            .without_legend()
                            .save();
                    "#,
                        backtrace_id.raw()
                    );

                    let args = cli_core::script::EngineArgs {
                        data: Some(data.clone()),
                        allocation_ids: Some(Arc::new(matched_allocation_ids.to_owned())),
                        ..cli_core::script::EngineArgs::default()
                    };

                    let mut urls = state.generate_graphs(&data, args, &code).into_iter();
                    only_matched.graph_url = urls.next();
                    only_matched.graph_preview_url = urls.next();
                }

                protocol::AllocationGroup {
                    all,
                    only_matched,
                    backtrace_id: backtrace_id.raw(),
                    backtrace,
                }
            })
    };

    let response = protocol::ResponseAllocationGroups {
        allocations: StreamingSerializer::new(factory),
        total_count: total_count as _,
    };

    response
}

async fn handler_allocation_groups(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter_params: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter_params, &custom_filter)?;
    let backtrace_format: protocol::BacktraceFormat = query(&req)?;
    let params: protocol::RequestAllocationGroups = query(&req)?;

    let key = AllocationGroupsKey {
        data_id: data.id(),
        filter: filter_params,
        custom_filter,
        sort_by: params
            .sort_by
            .unwrap_or(protocol::AllocGroupsSortBy::MinTimestamp),
        order: params.order.unwrap_or(protocol::Order::Asc),
    };

    let groups = req.state().allocation_group_cache.lock().get(&key).cloned();

    fn sort_by<T, F>(
        data: &Data,
        groups: &mut AllocationGroups,
        order: protocol::Order,
        is_global: bool,
        callback: F,
    ) where
        F: Fn(&protocol::AllocationGroupData) -> T + Send + Sync,
        T: Ord + Send + Sync,
    {
        if is_global {
            groups
                .allocations_by_backtrace
                .par_sort_by_key(|(&backtrace_id, _)| {
                    let group_data = get_global_group_data(data, backtrace_id);
                    callback(&group_data)
                });
        } else {
            let key_for_backtrace: Vec<_> = groups
                .allocations_by_backtrace
                .par_iter()
                .map(|(&backtrace_id, ids)| {
                    let allocations = ids.par_iter().map(|&id| data.get_allocation(id));
                    let group_data = get_allocation_group_data(data, allocations);
                    (backtrace_id, callback(&group_data))
                })
                .collect();

            let key_for_backtrace: HashMap<_, _> = key_for_backtrace.into_iter().collect();
            groups
                .allocations_by_backtrace
                .par_sort_by_key(|(&backtrace_id, _)| {
                    key_for_backtrace.get(&backtrace_id).unwrap().clone()
                });
        }

        match order {
            protocol::Order::Asc => {}
            protocol::Order::Dsc => {
                groups.allocations_by_backtrace.reverse();
            }
        }
    }

    let allocation_groups;
    if let Some(groups) = groups {
        allocation_groups = groups;
    } else {
        let iter = prefiltered_allocation_ids(data, Default::default(), &filter)
            .par_iter()
            .map(|&allocation_id| (allocation_id, data.get_allocation(allocation_id)))
            .filter(move |(id, allocation)| filter.try_match(data, *id, allocation));

        let mut groups = AllocationGroups::new(iter);
        match key.sort_by {
            protocol::AllocGroupsSortBy::MinTimestamp => {
                sort_by(data, &mut groups, key.order, false, |group_data| {
                    group_data.min_timestamp.clone()
                });
            }
            protocol::AllocGroupsSortBy::MaxTimestamp => {
                sort_by(data, &mut groups, key.order, false, |group_data| {
                    group_data.max_timestamp.clone()
                });
            }
            protocol::AllocGroupsSortBy::Interval => {
                sort_by(data, &mut groups, key.order, false, |group_data| {
                    group_data.interval.clone()
                });
            }
            protocol::AllocGroupsSortBy::AllocatedCount => {
                sort_by(data, &mut groups, key.order, false, |group_data| {
                    group_data.allocated_count
                });
            }
            protocol::AllocGroupsSortBy::LeakedCount => {
                sort_by(data, &mut groups, key.order, false, |group_data| {
                    group_data.leaked_count
                });
            }
            protocol::AllocGroupsSortBy::Size => {
                sort_by(data, &mut groups, key.order, false, |group_data| {
                    group_data.size
                });
            }
            protocol::AllocGroupsSortBy::GlobalMinTimestamp => {
                sort_by(data, &mut groups, key.order, true, |group_data| {
                    group_data.min_timestamp.clone()
                });
            }
            protocol::AllocGroupsSortBy::GlobalMaxTimestamp => {
                sort_by(data, &mut groups, key.order, true, |group_data| {
                    group_data.max_timestamp.clone()
                });
            }
            protocol::AllocGroupsSortBy::GlobalInterval => {
                sort_by(data, &mut groups, key.order, true, |group_data| {
                    group_data.interval.clone()
                });
            }
            protocol::AllocGroupsSortBy::GlobalAllocatedCount => {
                sort_by(data, &mut groups, key.order, true, |group_data| {
                    group_data.allocated_count
                });
            }
            protocol::AllocGroupsSortBy::GlobalLeakedCount => {
                sort_by(data, &mut groups, key.order, true, |group_data| {
                    group_data.leaked_count
                });
            }
            protocol::AllocGroupsSortBy::GlobalSize => {
                sort_by(data, &mut groups, key.order, true, |group_data| {
                    group_data.size
                });
            }
            protocol::AllocGroupsSortBy::GlobalMaxTotalUsageFirstSeenAt => {
                sort_by(data, &mut groups, key.order, true, |group_data| {
                    group_data.max_total_usage_first_seen_at.clone()
                });
            }
        }

        allocation_groups = Arc::new(groups);
        req.state()
            .allocation_group_cache
            .lock()
            .put(key, allocation_groups.clone());
    }

    let state = req.state().clone();
    let body = async_data_handler(&req, move |data, tx| {
        let response =
            get_allocation_groups(&state, &data, backtrace_format, params, allocation_groups);
        let _ = serde_json::to_writer(tx, &response);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(body))
}

async fn handler_raw_allocations(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let iter = data
        .alloc_sorted_by_timestamp(None, None)
        .iter()
        .map(|&id| data.get_allocation(id));

    let mut output = String::new();
    output.push_str("[");

    let mut is_first = true;
    for allocation in iter {
        if !is_first {
            output.push_str(",");
        } else {
            is_first = false;
        }

        output.push_str("{\"backtrace\":[");
        let mut is_first = true;
        for (_, frame) in data.get_backtrace(allocation.backtrace) {
            if !is_first {
                output.push_str(",");
            } else {
                is_first = false;
            }

            let address = frame.address().raw();
            write!(output, "\"{:016X}\"", address).unwrap();
        }
        output.push_str("]}");
    }

    output.push_str("]");
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(output))
}

fn dump_node<T: fmt::Write, K: PartialEq + Clone, V, F: Fn(&mut T, &V) -> fmt::Result>(
    tree: &Tree<K, V>,
    node_id: NodeId,
    output: &mut T,
    printer: &mut F,
) -> fmt::Result {
    write!(output, "{{")?;

    let node = tree.get_node(node_id);
    write!(output, "\"size\":{},", node.total_size)?;
    write!(output, "\"count\":{},", node.total_count)?;
    write!(
        output,
        "\"first\":{},",
        node.total_first_timestamp.as_secs()
    )?;
    write!(output, "\"last\":{},", node.total_last_timestamp.as_secs())?;
    if let Some(value) = node.value() {
        write!(output, "\"value\":")?;
        printer(output, value)?;
        write!(output, ",")?;
    }

    write!(output, "\"children\":[")?;
    for (index, &(_, child_id)) in tree.get_node(node_id).children.iter().enumerate() {
        if index != 0 {
            write!(output, ",")?;
        }

        dump_node(tree, child_id, output, printer)?;
    }
    write!(output, "]")?;

    write!(output, "}}")?;
    Ok(())
}

async fn handler_tree(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter, &custom_filter)?;
    let backtrace_format: protocol::BacktraceFormat = query(&req)?;

    let body = async_data_handler(&req, move |data, mut tx| {
        let mut tree: Tree<FrameId, &Frame> = Tree::new();
        for (allocation_id, allocation) in data.allocations_with_id() {
            if !filter.try_match(&data, allocation_id, allocation) {
                continue;
            }

            tree.add_allocation(
                allocation,
                allocation_id,
                data.get_backtrace(allocation.backtrace),
            );
        }

        dump_node(&tree, 0, &mut tx, &mut |output, frame| {
            let frame = get_frame(&data, &backtrace_format, frame);
            serde_json::to_writer(output, &frame).map_err(|_| fmt::Error)
        })
        .unwrap();
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(body))
}

async fn handler_backtrace(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let backtrace_id: u32 = req
        .match_info()
        .get("backtrace_id")
        .unwrap()
        .parse()
        .unwrap();
    let backtrace_id = BacktraceId::new(backtrace_id);
    let backtrace = data.get_backtrace(backtrace_id);
    let backtrace_format: protocol::BacktraceFormat = query(&req)?;

    let mut frames = Vec::new();
    for (_, frame) in backtrace {
        frames.push(get_frame(data, &backtrace_format, frame));
    }

    let response = protocol::ResponseBacktrace { frames };

    Ok(HttpResponse::Ok().json(response))
}

async fn handler_backtraces(req: HttpRequest) -> Result<HttpResponse> {
    let backtrace_format: protocol::BacktraceFormat = query(&req)?;
    let filter: protocol::BacktraceFilter = query(&req)?;
    let filter = crate::filter::prepare_backtrace_filter(&filter)?;
    let body = async_data_handler(&req, move |data, tx| {
        let mut positive_cache = HashMap::new();
        let mut negative_cache = HashMap::new();
        let total_count = data
            .all_backtraces()
            .flat_map(|(_, backtrace)| {
                if !crate::filter::match_backtrace(
                    &data,
                    &mut positive_cache,
                    &mut negative_cache,
                    &filter,
                    backtrace,
                ) {
                    None
                } else {
                    Some(())
                }
            })
            .count();

        let data = &data;
        let backtraces = move || {
            let mut positive_cache = positive_cache.clone();
            let mut negative_cache = negative_cache.clone();
            let backtrace_format = backtrace_format.clone();
            let filter = filter.clone();
            data.all_backtraces().flat_map(move |(_, backtrace)| {
                if !crate::filter::match_backtrace(
                    &data,
                    &mut positive_cache,
                    &mut negative_cache,
                    &filter,
                    backtrace.clone(),
                ) {
                    return None;
                }

                let mut frames = Vec::new();
                for (_, frame) in backtrace {
                    frames.push(get_frame(&data, &backtrace_format, frame));
                }
                Some(frames)
            })
        };

        let response = protocol::ResponseBacktraces {
            backtraces: StreamingSerializer::new(backtraces),
            total_count: total_count as u64,
        };

        let _ = serde_json::to_writer(tx, &response);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(body))
}

fn generate_regions<'a, F: Fn(AllocationId, &Allocation) -> bool + Clone + 'a>(
    data: &'a Data,
    filter: F,
) -> impl Serialize + 'a {
    let main_heap_start = data
        .alloc_sorted_by_address(None, None)
        .iter()
        .map(|&id| data.get_allocation(id))
        .filter(|allocation| !allocation.is_mmaped() && allocation.in_main_arena())
        .map(|allocation| allocation.actual_range(data).start)
        .next()
        .unwrap_or(0);

    let main_heap_end = data
        .alloc_sorted_by_address(None, None)
        .iter()
        .map(|&id| data.get_allocation(id))
        .rev()
        .filter(|allocation| !allocation.is_mmaped() && allocation.in_main_arena())
        .map(|allocation| allocation.actual_range(data).end)
        .next()
        .unwrap_or(0);

    let regions = move || {
        let filter = filter.clone();
        data.alloc_sorted_by_address(None, None)
            .iter()
            .map(move |&id| (id, data.get_allocation(id)))
            .filter(move |(id, allocation)| filter(*id, allocation))
            .map(move |(_, allocation)| allocation.actual_range(data))
            .coalesce(|mut range, next_range| {
                if next_range.start <= range.end {
                    range.end = next_range.end;
                    Ok(range)
                } else {
                    Err((range, next_range))
                }
            })
            .map(|range| [range.start, range.end])
    };

    protocol::ResponseRegions {
        main_heap_start,
        main_heap_end,
        main_heap_start_s: format!("{}", main_heap_start),
        main_heap_end_s: format!("{}", main_heap_end),
        regions: StreamingSerializer::new(regions),
    }
}

async fn handler_regions(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter, &custom_filter)?;

    let body = async_data_handler(&req, move |data, tx| {
        let response = generate_regions(&data, |id, allocation| {
            filter.try_match(&data, id, allocation)
        });
        let _ = serde_json::to_writer(tx, &response);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(body))
}

async fn handler_mallopts(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let backtrace_format: protocol::BacktraceFormat = query(&req)?;

    let response: Vec<_> = data
        .mallopts()
        .iter()
        .map(|mallopt| {
            let mut backtrace = Vec::new();
            for (_, frame) in data.get_backtrace(mallopt.backtrace) {
                backtrace.push(get_frame(&data, &backtrace_format, frame));
            }

            protocol::Mallopt {
                timestamp: mallopt.timestamp.into(),
                thread: mallopt.thread,
                backtrace_id: mallopt.backtrace.raw(),
                backtrace,
                raw_param: mallopt.kind.raw(),
                param: match mallopt.kind {
                    MalloptKind::TrimThreshold => Some("M_TRIM_THRESHOLD"),
                    MalloptKind::TopPad => Some("M_TOP_PAD"),
                    MalloptKind::MmapThreshold => Some("M_MMAP_THRESHOLD"),
                    MalloptKind::MmapMax => Some("M_MMAP_MAX"),
                    MalloptKind::CheckAction => Some("M_CHECK_ACTION"),
                    MalloptKind::Perturb => Some("M_PERTURB"),
                    MalloptKind::ArenaTest => Some("M_ARENA_TEXT"),
                    MalloptKind::ArenaMax => Some("M_ARENA_MAX"),
                    MalloptKind::Other(_) => None,
                }
                .map(|value| value.into()),
                value: mallopt.value,
                result: mallopt.result,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(response))
}

async fn handler_export_flamegraph_pl(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter, &custom_filter)?;

    let body = async_data_handler(&req, move |data, tx| {
        let _ = export_as_flamegraph_pl(&data, tx, |id, allocation| {
            filter.try_match(&data, id, allocation)
        });
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(body))
}

async fn handler_export_flamegraph(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter, &custom_filter)?;

    let body = async_data_handler(&req, move |data, tx| {
        let _ = export_as_flamegraph(&data, tx, |id, allocation| {
            filter.try_match(&data, id, allocation)
        });
    })?;

    Ok(HttpResponse::Ok().content_type("image/svg+xml").body(body))
}

async fn handler_export_replay(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter, &custom_filter)?;

    let body = async_data_handler(&req, move |data, tx| {
        let _ = export_as_replay(&data, tx, |id, allocation| {
            filter.try_match(&data, id, allocation)
        });
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(body))
}

async fn handler_export_heaptrack(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(data, &filter, &custom_filter)?;

    let body = async_data_handler(&req, move |data, tx| {
        let _ = export_as_heaptrack(&data, tx, |id, allocation| {
            filter.try_match(&data, id, allocation)
        });
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(body))
}

async fn handler_allocation_ascii_tree(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;
    let filter = prepare_allocation_filter(&data, &filter, &custom_filter)?;

    let body = async_data_handler(&req, move |data, mut tx| {
        let tree = data.tree_by_source(|id, allocation| filter.try_match(&data, id, allocation));
        let table = data.dump_tree(&tree);
        let table = table_to_string(&table);
        let _ = writeln!(tx, "{}", table);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(body))
}

async fn handler_collation_json<F>(req: HttpRequest, callback: F) -> Result<HttpResponse>
where
    F: Fn(&Data) -> BTreeMap<String, BTreeMap<u32, CountAndSize>> + Send + 'static,
{
    use serde_json::json;

    let body = async_data_handler(&req, move |data, tx| {
        let constants = callback(&data);
        let mut total_count = 0;
        let mut total_size = 0;
        let per_file: BTreeMap<_, _> = constants
            .into_iter()
            .map(|(key, per_line)| {
                let mut whole_file_count = 0;
                let mut whole_file_size = 0;
                let per_line: BTreeMap<_, _> = per_line
                    .into_iter()
                    .map(|(line, entry)| {
                        whole_file_count += entry.count;
                        whole_file_size += entry.size;
                        total_count += entry.count;
                        total_size += entry.size;
                        let entry = json!({
                            "count": entry.count,
                            "size": entry.size
                        });
                        (line, entry)
                    })
                    .collect();

                let value = json!({
                    "count": whole_file_count,
                    "size": whole_file_size,
                    "per_line": per_line
                });

                (key, value)
            })
            .collect();

        let response = json!({
            "count": total_count,
            "size": total_size,
            "per_file": per_file
        });

        let _ = serde_json::to_writer(tx, &response);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .body(body))
}

async fn handler_dynamic_constants(req: HttpRequest) -> Result<HttpResponse> {
    handler_collation_json(req, |data| data.get_dynamic_constants()).await
}

async fn handler_dynamic_statics(req: HttpRequest) -> Result<HttpResponse> {
    handler_collation_json(req, |data| data.get_dynamic_statics()).await
}

async fn handler_dynamic_constants_ascii_tree(req: HttpRequest) -> Result<HttpResponse> {
    let body = async_data_handler(&req, move |data, mut tx| {
        let table = data.get_dynamic_constants_ascii_tree();
        let _ = writeln!(tx, "{}", table);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(body))
}

async fn handler_dynamic_statics_ascii_tree(req: HttpRequest) -> Result<HttpResponse> {
    let body = async_data_handler(&req, move |data, mut tx| {
        let table = data.get_dynamic_statics_ascii_tree();
        let _ = writeln!(tx, "{}", table);
    })?;

    Ok(HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(body))
}

async fn handler_script_files(req: HttpRequest) -> Result<HttpResponse> {
    let hash = req.match_info().get("hash").unwrap();
    let entry = match req.state().generated_files.lock().by_hash.get(hash) {
        Some(entry) => entry.clone(),
        None => {
            return Err(ErrorNotFound("file not found"));
        }
    };

    let (mut tx, rx) = byte_channel();
    // let rx = rx.map_err(|_| ErrorInternalServerError("internal error"));
    let rx = BodyStream::new(rx);
    let body = BoxBody::new(rx);
    let mime = entry.mime;
    thread::spawn(move || {
        use std::io::Write;
        tx.write_all(&entry.data).unwrap();
    });

    Ok(HttpResponse::Ok().content_type(mime).body(body))
}

async fn handler_allocation_filter_to_script(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::AllocFilter = query(&req)?;
    let filter = prepare_raw_allocation_filter(data, &filter)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;

    let mut prologue = String::new();
    let code;

    if let Some(custom_filter) = custom_filter.custom_filter {
        prologue.push_str("fn custom_filter() {\n");
        for line in custom_filter.lines() {
            prologue.push_str("    ");
            prologue.push_str(line);
            prologue.push_str("\n");
        }
        prologue.push_str("}\n\n");
        prologue.push_str("let filtered = custom_filter();\n");
        code = filter.to_code(Some("filtered".into()));
    } else {
        code = filter.to_code(None);
    }

    let body = serde_json::json! {{
        "prologue": prologue,
        "code": code
    }}
    .to_string();

    Ok(HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .body(body))
}

async fn handler_map_filter_to_script(req: HttpRequest) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let filter: protocol::MapFilter = query(&req)?;
    let filter = prepare_raw_map_filter(data, &filter)?;
    let custom_filter: protocol::CustomFilter = query(&req)?;

    let mut prologue = String::new();
    let code;

    if let Some(custom_filter) = custom_filter.custom_filter {
        prologue.push_str("fn custom_filter() {\n");
        for line in custom_filter.lines() {
            prologue.push_str("    ");
            prologue.push_str(line);
            prologue.push_str("\n");
        }
        prologue.push_str("}\n\n");
        prologue.push_str("let filtered = custom_filter();\n");
        code = filter.to_code(Some("filtered".into()));
    } else {
        code = filter.to_code(None);
    }

    let body = serde_json::json! {{
        "prologue": prologue,
        "code": code
    }}
    .to_string();

    Ok(HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .body(body))
}

async fn handler_execute_script(req: HttpRequest, body: web::Bytes) -> Result<HttpResponse> {
    let data = get_data(&req)?;
    let body = String::from_utf8(body.to_vec()).unwrap();
    let args = cli_core::script::EngineArgs {
        data: Some(data.clone()),
        ..cli_core::script::EngineArgs::default()
    };

    let env = Arc::new(Mutex::new(cli_core::script::VirtualEnvironment::new()));
    let engine = cli_core::script::Engine::new(env.clone(), args);
    let timestamp = std::time::Instant::now();
    let result = engine.run(&body);
    let elapsed = timestamp.elapsed();
    let data_id = data.id();

    let mut new_files = Vec::new();
    let mut output = Vec::new();
    for item in std::mem::take(&mut env.lock().output) {
        match item {
            cli_core::script::ScriptOutputKind::PrintLine(line) => {
                output.push(serde_json::json! {{
                    "kind": "println",
                    "value": line
                }});
            }
            cli_core::script::ScriptOutputKind::Image { path, data } => {
                let hash = format!("{:x}", md5::compute(&*data));
                let basename = path[path.rfind("/").unwrap() + 1..].to_owned();
                output.push(serde_json::json! {{
                    "url": format!( "/data/{}/script_files/{}/{}", data_id, hash, basename ),
                    "kind": "image",
                    "basename": basename,
                    "path": path,
                    "checksum": hash
                }});

                let entry = GeneratedFile {
                    timestamp: Instant::now(),
                    hash,
                    mime: "image/svg+xml",
                    data,
                };

                new_files.push(entry);
            }
        }
    }

    let mut generated = req.state().generated_files.lock();
    generated.purge_old_if_too_big();
    for entry in new_files {
        generated.add_file(entry);
    }
    std::mem::drop(generated);

    let result = match result {
        Ok(_) => {
            serde_json::json! {{
                "status": "ok",
                "elapsed": elapsed.as_secs_f64(),
                "output": output
            }}
        }
        Err(error) => {
            serde_json::json! {{
                "status": "error",
                "message": error.message,
                "line": error.line,
                "column": error.column,
                "output": output
            }}
        }
    };

    Ok(HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .append_header(("Access-Control-Allow-Origin", "http://localhost:1234"))
        .body(serde_json::to_string(&result).unwrap()))
}

fn guess_mime(path: &str) -> &str {
    macro_rules! mimes {
        ($($ext:expr => $mime:expr),+) => {
            $(
                if path.ends_with( $ext ) { return $mime; }
            )+
        };
    }

    mimes! {
        ".html" => "text/html",
        ".css" => "text/css",
        ".js" => "text/javascript",
        ".svg" => "image/svg+xml",
        ".woff" => "font/woff",
        ".woff2" => "font/woff2",
        ".ttf" => "font/ttf",
        ".eot" => "application/vnd.ms-fontobject"
    }

    "application/octet-stream"
}

#[derive(Clone)]
struct StaticResponse(&'static str, &'static [u8]);

impl StaticResponse {
    pub(crate) async fn respond(self) -> HttpResponse {
        HttpResponse::Ok()
            .content_type(guess_mime(self.0))
            .body(BoxBody::new(self.1))
    }
}
include!(concat!(env!("OUT_DIR"), "/webui_assets.rs"));

#[derive(Debug)]
pub enum ServerError {
    BindFailed(io::Error),
    Other(io::Error),
}

impl fmt::Display for ServerError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ServerError::BindFailed(ref error) if error.kind() == io::ErrorKind::AddrInUse => {
                write!( fmt, "cannot server the server: address is already in use; you probably want to pick a different port with `--port`" )
            }
            ServerError::BindFailed(ref error) => {
                write!(fmt, "failed to start the server: {}", error)
            }
            ServerError::Other(ref error) => write!(fmt, "{}", error),
        }
    }
}

impl From<io::Error> for ServerError {
    fn from(error: io::Error) -> Self {
        ServerError::Other(error)
    }
}

impl Error for ServerError {}
pub async fn server_main(
    inputs: Vec<PathBuf>,
    debug_symbols: Vec<PathBuf>,
    load_in_parallel: bool,
    interface: &str,
    port: u16,
) -> Result<(), ServerError> {
    let mut state = State::new();

    if !load_in_parallel {
        for filename in inputs {
            info!("Trying to load {:?}...", filename);
            let fp = File::open(filename)?;
            let data = Loader::load_from_stream(fp, &debug_symbols)?;
            state.add_data(data);
        }
    } else {
        let handles: Vec<thread::JoinHandle<io::Result<Data>>> = inputs
            .iter()
            .map(move |filename| {
                let filename = filename.clone();
                let debug_symbols = debug_symbols.clone();
                thread::spawn(move || {
                    info!("Trying to load {:?}...", filename);
                    let fp = File::open(filename)?;
                    let data = Loader::load_from_stream(fp, debug_symbols)?;
                    Ok(data)
                })
            })
            .collect();

        for handle in handles {
            let data = handle.join().unwrap()?;
            state.add_data(data);
        }
    }

    for (key, bytes) in WEBUI_ASSETS {
        debug!("Static asset: '{}', length = {}", key, bytes.len());
    }

    let state = Arc::new(state);
    actix_web::HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .wrap(Cors::permissive())
            .configure(|app| {
                app.service(web::resource("/list").route(web::get().to(handler_list)))
                    .service(
                        web::resource("/data/{id}/timeline").route(web::get().to(handler_timeline)),
                    )
                    .service(
                        web::resource("/data/{id}/timeline_leaked")
                            .route(web::get().to(handler_timeline_leaked)),
                    )
                    .service(
                        web::resource("/data/{id}/timeline_maps")
                            .route(web::get().to(handler_timeline_maps)),
                    )
                    .service(
                        web::resource("/data/{id}/allocations")
                            .route(web::get().to(handler_allocations)),
                    )
                    .service(
                        web::resource("/data/{id}/allocation_groups")
                            .route(web::get().to(handler_allocation_groups)),
                    )
                    .service(web::resource("/data/{id}/maps").route(web::get().to(handler_maps)))
                    .service(
                        web::resource("/data/{id}/backtraces")
                            .route(web::get().to(handler_backtraces)),
                    )
                    .service(
                        web::resource("/data/{id}/raw_allocations")
                            .route(web::get().to(handler_raw_allocations)),
                    )
                    .service(web::resource("/data/{id}/tree").route(web::get().to(handler_tree)))
                    .service(
                        web::resource("/data/{id}/backtrace/{backtrace_id}")
                            .route(web::get().to(handler_backtrace)),
                    )
                    .service(
                        web::resource("/data/{id}/regions").route(web::get().to(handler_regions)),
                    )
                    .service(
                        web::resource("/data/{id}/mallopts").route(web::get().to(handler_mallopts)),
                    )
                    .service(
                        web::resource("/data/{id}/export/flamegraph")
                            .route(web::get().to(handler_export_flamegraph)),
                    )
                    .service(
                        web::resource("/data/{id}/export/flamegraph/{filename}")
                            .route(web::get().to(handler_export_flamegraph)),
                    )
                    .service(
                        web::resource("/data/{id}/export/flamegraph.pl")
                            .route(web::get().to(handler_export_flamegraph_pl)),
                    )
                    .service(
                        web::resource("/data/{id}/export/flamegraph.pl/{filename}")
                            .route(web::get().to(handler_export_flamegraph_pl)),
                    )
                    .service(
                        web::resource("/data/{id}/export/heaptrack")
                            .route(web::get().to(handler_export_heaptrack)),
                    )
                    .service(
                        web::resource("/data/{id}/export/heaptrack/{filename}")
                            .route(web::get().to(handler_export_heaptrack)),
                    )
                    .service(
                        web::resource("/data/{id}/export/replay")
                            .route(web::get().to(handler_export_replay)),
                    )
                    .service(
                        web::resource("/data/{id}/export/replay/{filename}")
                            .route(web::get().to(handler_export_replay)),
                    )
                    .service(
                        web::resource("/data/{id}/allocation_ascii_tree")
                            .route(web::get().to(handler_allocation_ascii_tree)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_constants")
                            .route(web::get().to(handler_dynamic_constants)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_constants/{filename}")
                            .route(web::get().to(handler_dynamic_constants)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_constants_ascii_tree")
                            .route(web::get().to(handler_dynamic_constants_ascii_tree)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_constants_ascii_tree/{filename}")
                            .route(web::get().to(handler_dynamic_constants_ascii_tree)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_statics")
                            .route(web::get().to(handler_dynamic_statics)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_statics/{filename}")
                            .route(web::get().to(handler_dynamic_statics)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_statics_ascii_tree")
                            .route(web::get().to(handler_dynamic_statics_ascii_tree)),
                    )
                    .service(
                        web::resource("/data/{id}/dynamic_statics_ascii_tree/{filename}")
                            .route(web::get().to(handler_dynamic_statics_ascii_tree)),
                    )
                    .service(
                        web::resource("/data/{id}/execute_script")
                            .route(web::post().to(handler_execute_script)),
                    )
                    .service(
                        web::resource("/data/{id}/script_files/{hash}/{filename}")
                            .route(web::get().to(handler_script_files)),
                    )
                    .service(
                        web::resource("/data/{id}/allocation_filter_to_script")
                            .route(web::get().to(handler_allocation_filter_to_script)),
                    )
                    .service(
                        web::resource("/data/{id}/map_filter_to_script")
                            .route(web::get().to(handler_map_filter_to_script)),
                    );

                for (key, bytes) in WEBUI_ASSETS {
                    // app.service(
                    //     web::resource(&format!("/{}", key))
                    //         .route(web::get().to::<_, HttpRequest>(StaticResponse(key, bytes))),
                    // );
                    app.service(web::resource(&format!("/{}", key)).route(web::get().to(
                        move || {
                            let res = StaticResponse(key, bytes);
                            res.respond()
                        },
                    )));
                    if *key == "index.html" {
                        app.service(web::resource("/").route(web::get().to(move || {
                            let res = StaticResponse(key, bytes);
                            res.respond()
                        })));
                    }
                }
            })
    })
    .bind(&format!("{}:{}", interface, port))
    .map_err(|err| ServerError::BindFailed(err))?
    .shutdown_timeout(1)
    .run()
    .await?;

    Ok(())
}
