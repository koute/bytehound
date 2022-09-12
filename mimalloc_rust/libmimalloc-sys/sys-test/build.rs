fn main() {
    let mut cfg = ctest2::TestGenerator::new();
    cfg.header("mimalloc.h")
        .include(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../c_src/mimalloc/include"
        ))
        .cfg("feature", Some("extended"))
        .fn_cname(|rust, link_name| link_name.unwrap_or(rust).to_string())
        // ignore whether or not the option enum is signed.
        .skip_signededness(|c| c.ends_with("_t") || c.ends_with("_e"))
        .type_name(|ty, _is_struct, _is_union| {
            match ty {
                // Special cases. We do this to avoid having both
                // `mi_blah_{s,e}` and `mi_blah_t`.
                "mi_heap_area_t" => "struct mi_heap_area_s".into(),
                "mi_heap_t" => "struct mi_heap_s".into(),
                "mi_options_t" => "enum mi_options_e".into(),

                // This also works but requires we export `mi_heap_s` and similar
                // in addition, so we just hardcode the above.

                // t if t.ends_with("_s") => format!("struct {}", t),
                // t if t.ends_with("_e") => format!("enum {}", t),
                // t if t.ends_with("_t") => t.to_string(),

                // mimalloc defines it's callbacks with the pointer at the
                // location of use, e.g. `typedef ret mi_some_fun(a0 x, a1 y);`
                // and then uses `mi_some_fun *arg` as argument types, which
                // appears to upset ctest, which would prefer function pointers
                // be declared as pointers, so we clean things up for it.
                t if t.ends_with("_fun") => format!("{}*", t),

                t => t.to_string(),
            }
        });

    cfg.generate("../src/lib.rs", "all.rs");
}
