use std::env;

fn main() {
    let mut build = cc::Build::new();

    build.include("c_src/mimalloc/include");
    build.include("c_src/mimalloc/src");
    build.file("c_src/mimalloc/src/static.c");

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("target_os not defined!");
    let target_family = env::var("CARGO_CFG_TARGET_FAMILY").expect("target_family not defined!");

    if env::var_os("CARGO_FEATURE_OVERRIDE").is_some() {
        // Overriding malloc is only available on windows in shared mode, but we
        // only ever build a static lib.
        if target_family != "windows" {
            build.define("MI_MALLOC_OVERRIDE", None);
        }
    }

    if env::var_os("CARGO_FEATURE_SECURE").is_some() {
        build.define("MI_SECURE", "4");
    }

    let dynamic_tls = env::var("CARGO_FEATURE_LOCAL_DYNAMIC_TLS").is_ok();

    if target_family == "unix" && target_os != "haiku" {
        if dynamic_tls {
            build.flag_if_supported("-ftls-model=local-dynamic");
        } else {
            build.flag_if_supported("-ftls-model=initial-exec");
        }
    }

    if env::var_os("CARGO_FEATURE_DEBUG").is_some()
        || (env::var_os("CARGO_FEATURE_DEBUG_IN_DEBUG").is_some() && cfg!(debug_assertions))
    {
        build.define("MI_DEBUG", "3");
        build.define("MI_SHOW_ERRORS", "1");
    } else {
        // Remove heavy debug assertions etc
        build.define("MI_DEBUG", "0");
    }

    if build.get_compiler().is_like_msvc() {
        build.cpp(true);
    }

    build.compile("mimalloc");
}
