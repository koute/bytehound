#[macro_use]
extern crate log;
#[macro_use]
extern crate bitflags;

#[cfg(test)]
#[macro_use]
extern crate quickcheck;

pub mod cmd_analyze_size;
pub mod cmd_extract;
pub mod cmd_gather;

mod data;
mod exporter_flamegraph;
mod exporter_flamegraph_pl;
mod exporter_heaptrack;
mod exporter_replay;
mod filter;
mod frame;
mod io_adapter;
mod loader;
mod postprocessor;
mod reader;
mod repack;
pub mod script;
mod script_virtual;
mod squeeze;
mod threaded_lz4_stream;
mod timeline;
mod tree;
mod tree_printer;
mod util;
mod vecvec;

pub use crate::data::{
    Allocation, AllocationId, BacktraceId, CodePointer, CountAndSize, Data, DataId, DataPointer,
    FrameId, Mallopt, MalloptKind, Map, MapId, MapUsage, Operation, OperationId, RegionFlags,
    StringId, Timestamp, UsageDelta,
};
pub use crate::exporter_flamegraph::export_as_flamegraph;
pub use crate::exporter_flamegraph_pl::export_as_flamegraph_pl;
pub use crate::exporter_heaptrack::export_as_heaptrack;
pub use crate::exporter_replay::export_as_replay;
pub use crate::frame::Frame;
pub use crate::loader::Loader;
pub use crate::postprocessor::{postprocess, Anonymize};
pub use crate::reader::parse_events;
pub use crate::repack::repack;
pub use crate::script::{run_script, EvalOutput};
pub use crate::squeeze::squeeze_data;
pub use crate::timeline::{build_allocation_timeline, build_map_timeline};
pub use crate::tree::{Node, NodeId, Tree};
pub use crate::util::table_to_string;
pub use crate::vecvec::VecVec;

pub use common::event;

pub use crate::filter::{
    AllocationFilter, Compile, CompiledAllocationFilter, CompiledMapFilter, Duration, Filter,
    MapFilter, NumberOrFractionOfTotal, RawAllocationFilter, RawCompiledAllocationFilter,
    RawCompiledMapFilter, RawMapFilter, TryMatch,
};
