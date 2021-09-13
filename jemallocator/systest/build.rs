use std::env;
use std::path::PathBuf;

const FUNCTION_POINTER: &[&str] = &[
    "extent_alloc_t",
    "extent_dalloc_t",
    "extent_destroy_t",
    "extent_commit_t",
    "extent_decommit_t",
    "extent_purge_t",
    "extent_split_t",
    "extent_merge_t",
];

fn main() {
    let root = PathBuf::from(env::var_os("DEP_JEMALLOC_ROOT").unwrap());

    let mut cfg = ctest::TestGenerator::new();
    cfg.header("jemalloc/jemalloc.h")
        .include(root.join("include"))
        .cfg("prefixed", None)
        .fn_cname(|rust, link_name| link_name.unwrap_or(rust).to_string())
        .skip_signededness(|c| c.ends_with("_t"))
        // No need to test pure C function pointer.
        .skip_type(|name| FUNCTION_POINTER.contains(&name));

    if cfg!(target_os = "linux") {
        cfg.skip_fn(|f| f == "malloc_usable_size");
    }

    cfg.generate("../jemalloc-sys/src/lib.rs", "all.rs");
}
