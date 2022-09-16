#![no_main]
use libfuzzer_sys::fuzz_target;

extern crate fast_range_map;

fuzz_target!( |data: &[u8]| {
    let mut map = fast_range_map::RangeMap::new();
    let mut map_sanity = rangemap::RangeMap::new();

    for (nth, chunk) in data.chunks_exact( 2 ).enumerate() {
        let start = chunk[0] as u64;
        let length = std::cmp::max( 1, chunk[1] as u64 );
        let end = start + length;
        let range = start..end;
        map.insert( range.clone(), nth );
        map_sanity.insert( range.clone(), nth );
    }

    let map: Vec< _ > = map.into_vec();
    let map_sanity: Vec< _ > = map_sanity.into_iter().collect();
    assert_eq!( map, map_sanity );
});
