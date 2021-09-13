# jemalloc-ctl

[![Travis-CI Status]][travis] [![Latest Version]][crates.io] [![docs]][docs.rs]

> A safe wrapper over `jemalloc`'s `mallctl*()` control and introspection APIs.

## Documentation

* [Latest release (docs.rs)][docs.rs]

## Platform support

Supported on all platforms supported by the [`tikv-jemallocator`] crate.

## Example

```no_run

use std::thread;
use std::time::Duration;
use tikv_jemalloc_ctl::{stats, epoch};

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() {
    // Obtain a MIB for the `epoch`, `stats.allocated`, and
    // `atats.resident` keys:
    let e = epoch::mib().unwrap();
    let allocated = stats::allocated::mib().unwrap();
    let resident = stats::resident::mib().unwrap();
    
    loop {
        // Many statistics are cached and only updated 
        // when the epoch is advanced:
        e.advance().unwrap();
        
        // Read statistics using MIB key:
        let allocated = allocated.read().unwrap();
        let resident = resident.read().unwrap();
        println!("{} bytes allocated/{} bytes resident", allocated, resident);
        thread::sleep(Duration::from_secs(10));
    }
}
```

## License

This project is licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `jemalloc-ctl` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[`tikv-jemallocator`]: https://github.com/tikv/jemallocator
[travis]: https://travis-ci.com/tikv/jemallocator
[Travis-CI Status]: https://travis-ci.com/tikv/jemallocator.svg?branch=master
[Latest Version]: https://img.shields.io/crates/v/tikv-jemallocator.svg
[crates.io]: https://crates.io/crates/tikv-jemallocator
[docs]: https://docs.rs/tikv-jemallocator/badge.svg
[docs.rs]: https://docs.rs/tikv-jemallocator/
