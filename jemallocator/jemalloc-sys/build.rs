// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// `jemalloc` is known not to work on these targets:
const UNSUPPORTED_TARGETS: &[&str] = &[
    "rumprun",
    "bitrig",
    "emscripten",
    "fuchsia",
    "redox",
    "wasm32",
];

// `jemalloc-sys` is not tested on these targets in CI:
const UNTESTED_TARGETS: &[&str] = &["openbsd", "msvc"];

// `jemalloc`'s background_thread support is known not to work on these targets:
const NO_BG_THREAD_TARGETS: &[&str] = &["musl"];

// targets that don't support unprefixed `malloc`
//
// “it was found that the `realpath` function in libc would allocate with libc malloc
//  (not jemalloc malloc), and then the standard library would free with jemalloc free,
//  causing a segfault.”
// https://github.com/rust-lang/rust/commit/e3b414d8612314e74e2b0ebde1ed5c6997d28e8d
// https://github.com/rust-lang/rust/commit/536011d929ecbd1170baf34e09580e567c971f95
// https://github.com/rust-lang/rust/commit/9f3de647326fbe50e0e283b9018ab7c41abccde3
// https://github.com/rust-lang/rust/commit/ed015456a114ae907a36af80c06f81ea93182a24
const NO_UNPREFIXED_MALLOC: &[&str] = &["android", "dragonfly", "musl", "darwin"];

macro_rules! info {
    ($($args:tt)*) => { println!($($args)*) }
}

macro_rules! warning {
    ($arg:tt, $($args:tt)*) => {
        println!(concat!(concat!("cargo:warning=\"", $arg), "\""), $($args)*)
    }
}

// TODO: split main functions and remove following allow.
#[allow(clippy::cognitive_complexity)]
fn main() {
    let target = env::var("TARGET").expect("TARGET was not set");
    let host = env::var("HOST").expect("HOST was not set");
    let num_jobs = env::var("NUM_JOBS").expect("NUM_JOBS was not set");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR was not set"));
    let src_dir = env::current_dir().expect("failed to get current directory");

    info!("TARGET={}", target);
    info!("HOST={}", host);
    info!("NUM_JOBS={}", num_jobs);
    info!("OUT_DIR={:?}", out_dir);
    let build_dir = out_dir.join("build");
    info!("BUILD_DIR={:?}", build_dir);
    info!("SRC_DIR={:?}", src_dir);

    if UNSUPPORTED_TARGETS.iter().any(|i| target.contains(i)) {
        panic!("jemalloc does not support target: {}", target);
    }

    if UNTESTED_TARGETS.iter().any(|i| target.contains(i)) {
        warning!("jemalloc support for `{}` is untested", target);
    }

    let mut use_prefix =
        env::var("CARGO_FEATURE_UNPREFIXED_MALLOC_ON_SUPPORTED_PLATFORMS").is_err();

    if !use_prefix && NO_UNPREFIXED_MALLOC.iter().any(|i| target.contains(i)) {
        warning!(
            "Unprefixed `malloc` requested on unsupported platform `{}` => using prefixed `malloc`",
            target
        );
        use_prefix = true;
    }

    // this has to occur before the early return when JEMALLOC_OVERRIDE is set
    if use_prefix {
        println!("cargo:rustc-cfg=prefixed");
    }

    println!("cargo:rustc-link-lib={}={}", "dylib", "preload_syscallee");
    if let Some(jemalloc) = env::var_os("JEMALLOC_OVERRIDE") {
        info!("jemalloc override set");
        let jemalloc = PathBuf::from(jemalloc);
        assert!(
            jemalloc.exists(),
            "Path to `jemalloc` in `JEMALLOC_OVERRIDE={}` does not exist",
            jemalloc.display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            jemalloc.parent().unwrap().display()
        );
        let stem = jemalloc.file_stem().unwrap().to_str().unwrap();
        let name = jemalloc.file_name().unwrap().to_str().unwrap();
        let kind = if name.ends_with(".a") {
            "static"
        } else {
            "dylib"
        };
        println!("cargo:rustc-link-lib={}={}", kind, &stem[3..]);
        return;
    }
    // Disable -Wextra warnings - jemalloc doesn't compile free of warnings with
    // it enabled: https://github.com/jemalloc/jemalloc/issues/1196
    let compiler = cc::Build::new().extra_warnings(false).get_compiler();
    let cflags = compiler
        .args()
        .iter()
        .map(|s| s.to_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    info!("CC={:?}", compiler.path());
    info!("CFLAGS={:?}", cflags);

    assert!(out_dir.exists(), "OUT_DIR does not exist");
    let jemalloc_repo_dir = PathBuf::from("jemalloc");
    info!("JEMALLOC_REPO_DIR={:?}", jemalloc_repo_dir);

    if build_dir.exists() {
        fs::remove_dir_all(build_dir.clone()).unwrap();
    }
    // Copy jemalloc submodule to the OUT_DIR
    let mut copy_options = fs_extra::dir::CopyOptions::new();
    copy_options.overwrite = true;
    copy_options.copy_inside = true;
    fs_extra::dir::copy(&jemalloc_repo_dir, &build_dir, &copy_options)
        .expect("failed to copy jemalloc source code to OUT_DIR");
    assert!(build_dir.exists());

    // Configuration files
    let config_files = ["configure", "VERSION"];

    // Copy the configuration files to jemalloc's source directory
    for f in &config_files {
        fs::copy(Path::new("configure").join(f), build_dir.join(f))
            .expect("failed to copy config file to OUT_DIR");
    }

    // Run configure:
    let configure = build_dir.join("configure");
    let mut cmd = Command::new("sh");
    cmd.arg(
        configure
            .to_str()
            .unwrap()
            .replace("C:\\", "/c/")
            .replace("\\", "/"),
    )
    .current_dir(&build_dir)
    .env("CC", compiler.path())
    .env("CFLAGS", cflags.clone())
    .env("LDFLAGS", cflags.clone())
    .env("CPPFLAGS", cflags)
    .arg("--disable-cxx");

    if target.contains("ios") {
        // newer iOS deviced have 16kb page sizes:
        // closed: https://github.com/gnzlbg/jemallocator/issues/68
        cmd.arg("--with-lg-page=14");
    }

    // collect `malloc_conf` string:
    let mut malloc_conf = String::new();

    if let Some(bg) = BackgroundThreadSupport::new(&target) {
        // `jemalloc` is compiled with background thread run-time support on
        // available platforms by default so there is nothing to do to enable
        // it.

        if bg.always_enabled {
            // Background thread support does not enable background threads at
            // run-time, just support for enabling them via run-time configuration
            // options (they are disabled by default)

            // The `enable_background_threads` cargo feature forces background
            // threads to be enabled at run-time by default:
            malloc_conf += "background_thread:true";
        }
    } else {
        // Background thread run-time support is disabled by
        // disabling background threads at compile-time:
        malloc_conf += "background_thread:false";
    }

    if let Ok(malloc_conf_opts) = env::var("JEMALLOC_SYS_WITH_MALLOC_CONF") {
        malloc_conf += &format!(
            "{}{}",
            if malloc_conf.is_empty() { "" } else { "," },
            malloc_conf_opts
        );
    }

    if !malloc_conf.is_empty() {
        info!("--with-malloc-conf={}", malloc_conf);
        cmd.arg(format!("--with-malloc-conf={}", malloc_conf));
    }

    if let Ok(lg_page) = env::var("JEMALLOC_SYS_WITH_LG_PAGE") {
        info!("--with-lg-page={}", lg_page);
        cmd.arg(format!("--with-lg-page={}", lg_page));
    }

    if let Ok(lg_hugepage) = env::var("JEMALLOC_SYS_WITH_LG_HUGEPAGE") {
        info!("--with-lg-hugepage={}", lg_hugepage);
        cmd.arg(format!("--with-lg-hugepage={}", lg_hugepage));
    }

    if let Ok(lg_quantum) = env::var("JEMALLOC_SYS_WITH_LG_QUANTUM") {
        info!("--with-lg-quantum={}", lg_quantum);
        cmd.arg(format!("--with-lg-quantum={}", lg_quantum));
    }

    if let Ok(lg_vaddr) = env::var("JEMALLOC_SYS_WITH_LG_VADDR") {
        info!("--with-lg-vaddr={}", lg_vaddr);
        cmd.arg(format!("--with-lg-vaddr={}", lg_vaddr));
    }

    if use_prefix {
        cmd.arg("--with-jemalloc-prefix=_rjem_mp_");
        info!("--with-jemalloc-prefix=_rjem_mp_");
    }

    cmd.arg("--with-private-namespace=_rjem_mp_");

    if env::var("CARGO_FEATURE_DEBUG").is_ok() {
        info!("CARGO_FEATURE_DEBUG set");
        cmd.arg("--enable-debug");
    }

    if env::var("CARGO_FEATURE_PROFILING").is_ok() {
        info!("CARGO_FEATURE_PROFILING set");
        cmd.arg("--enable-prof");
    }

    if env::var("CARGO_FEATURE_STATS").is_ok() {
        info!("CARGO_FEATURE_STATS set");
        cmd.arg("--enable-stats");
    }

    if env::var("CARGO_FEATURE_DISABLE_INITIAL_EXEC_TLS").is_ok() {
        info!("CARGO_FEATURE_DISABLE_INITIAL_EXEC_TLS set");
        cmd.arg("--disable-initial-exec-tls");
    }

    cmd.arg(format!("--host={}", gnu_target(&target)));
    cmd.arg(format!("--build={}", gnu_target(&host)));
    cmd.arg(format!("--prefix={}", out_dir.display()));

    run_and_log(&mut cmd, &build_dir.join("config.log"));

    // Make:
    let make = make_cmd(&host);
    run(Command::new(make)
        .current_dir(&build_dir)
        .arg("-j")
        .arg(num_jobs.clone()));

    if env::var("JEMALLOC_SYS_RUN_JEMALLOC_TESTS").is_ok() {
        info!("Building and running jemalloc tests...");
        // Make tests:
        run(Command::new(make)
            .current_dir(&build_dir)
            .arg("-j")
            .arg(num_jobs.clone())
            .arg("tests"));

        // Run tests:
        run(Command::new(make).current_dir(&build_dir).arg("check"));
    }

    // Make install:
    run(Command::new(make)
        .current_dir(&build_dir)
        .arg("install_lib_static")
        .arg("install_include")
        .arg("-j")
        .arg(num_jobs));

    println!("cargo:root={}", out_dir.display());

    // Linkage directives to pull in jemalloc and its dependencies.
    //
    // On some platforms we need to be sure to link in `pthread` which jemalloc
    // depends on, and specifically on android we need to also link to libgcc.
    // Currently jemalloc is compiled with gcc which will generate calls to
    // intrinsics that are libgcc specific (e.g. those intrinsics aren't present in
    // libcompiler-rt), so link that in to get that support.
    if target.contains("windows") {
        println!("cargo:rustc-link-lib=static=jemalloc");
    } else {
        println!("cargo:rustc-link-lib=static=jemalloc_pic");
    }
    println!("cargo:rustc-link-search=native={}/lib", build_dir.display());
    if target.contains("android") {
        println!("cargo:rustc-link-lib=gcc");
    } else if !target.contains("windows") {
        println!("cargo:rustc-link-lib=pthread");
    }
    println!("cargo:rerun-if-changed=jemalloc");
}

fn run_and_log(cmd: &mut Command, log_file: &Path) {
    execute(cmd, || {
        run(Command::new("tail").arg("-n").arg("100").arg(log_file));
    })
}

fn run(cmd: &mut Command) {
    execute(cmd, || ());
}

fn execute(cmd: &mut Command, on_fail: impl FnOnce()) {
    println!("running: {:?}", cmd);
    let status = match cmd.status() {
        Ok(status) => status,
        Err(e) => panic!("failed to execute command: {}", e),
    };
    if !status.success() {
        on_fail();
        panic!(
            "command did not execute successfully: {:?}\n\
             expected success, got: {}",
            cmd, status
        );
    }
}

fn gnu_target(target: &str) -> String {
    match target {
        "i686-pc-windows-msvc" => "i686-pc-win32".to_string(),
        "x86_64-pc-windows-msvc" => "x86_64-pc-win32".to_string(),
        "i686-pc-windows-gnu" => "i686-w64-mingw32".to_string(),
        "x86_64-pc-windows-gnu" => "x86_64-w64-mingw32".to_string(),
        "armv7-linux-androideabi" => "arm-linux-androideabi".to_string(),
        s => s.to_string(),
    }
}

fn make_cmd(host: &str) -> &'static str {
    const GMAKE_HOSTS: &[&str] = &["bitrig", "dragonfly", "freebsd", "netbsd", "openbsd"];
    if GMAKE_HOSTS.iter().any(|i| host.contains(i)) {
        "gmake"
    } else if host.contains("windows") {
        "mingw32-make"
    } else {
        "make"
    }
}

struct BackgroundThreadSupport {
    always_enabled: bool,
}

impl BackgroundThreadSupport {
    fn new(target: &str) -> Option<Self> {
        let runtime_support = env::var("CARGO_FEATURE_BACKGROUND_THREADS_RUNTIME_SUPPORT").is_ok();
        let always_enabled = env::var("CARGO_FEATURE_BACKGROUND_THREADS").is_ok();

        if !runtime_support {
            assert!(
                !always_enabled,
                "enabling `background_threads` requires `background_threads_runtime_support`"
            );
            return None;
        }

        if NO_BG_THREAD_TARGETS.iter().any(|i| target.contains(i)) {
            warning!(
                "`background_threads_runtime_support` not supported for `{}`",
                target
            );
        }

        Some(Self { always_enabled })
    }
}
