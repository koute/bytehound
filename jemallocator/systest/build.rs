extern crate ctest;

use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var_os("DEP_JEMALLOC_ROOT").unwrap());

    let mut cfg = ctest::TestGenerator::new();
    cfg.header("jemalloc/jemalloc.h")
       .include(root.join("include"))
       .cfg("prefixed", None)
       .fn_cname(|rust, link_name| link_name.unwrap_or(rust).to_string())
       .skip_signededness(|c| c.ends_with("_t"));

    if cfg!(target_os = "linux") {
        cfg.skip_fn(|f| f == "malloc_usable_size");
    }

    cfg.generate("../jemalloc-sys/src/lib.rs", "all.rs");
}
