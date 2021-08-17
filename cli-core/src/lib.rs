#[macro_use]
extern crate log;
#[macro_use]
extern crate bitflags;

#[cfg(test)]
#[macro_use]
extern crate quickcheck;

pub mod cmd_gather;
pub mod cmd_analyze_size;

mod filter;
mod util;
mod tree;
mod tree_printer;
mod reader;
mod loader;
mod postprocessor;
mod squeeze;
mod frame;
mod data;
mod io_adapter;
mod exporter_replay;
mod exporter_heaptrack;
mod exporter_flamegraph;
mod exporter_flamegraph_pl;
mod vecvec;
mod threaded_lz4_stream;
mod repack;
mod timeline;
pub mod script;
mod script_virtual;

pub use crate::data::{Data, DataId, CodePointer, DataPointer, BacktraceId, Timestamp, Operation, OperationId, StringId, Allocation, AllocationId, FrameId, Mallopt, MalloptKind, MmapOperation, MemoryMap, MemoryUnmap, CountAndSize};
pub use crate::loader::Loader;
pub use crate::tree::{Tree, Node, NodeId};
pub use crate::frame::Frame;
pub use crate::exporter_replay::export_as_replay;
pub use crate::exporter_heaptrack::export_as_heaptrack;
pub use crate::exporter_flamegraph_pl::export_as_flamegraph_pl;
pub use crate::exporter_flamegraph::export_as_flamegraph;
pub use crate::vecvec::VecVec;
pub use crate::util::table_to_string;
pub use crate::postprocessor::{Anonymize, postprocess};
pub use crate::squeeze::squeeze_data;
pub use crate::reader::parse_events;
pub use crate::repack::repack;
pub use crate::script::run_script;
pub use crate::timeline::build_timeline;

pub use common::event;

pub use crate::filter::{
    BasicFilter,
    CompiledFilter,
    Duration,
    Filter,
    NumberOrFractionOfTotal
};
