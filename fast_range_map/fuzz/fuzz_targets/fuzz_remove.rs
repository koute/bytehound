#![no_main]
use libfuzzer_sys::fuzz_target;

extern crate fast_range_map;

fuzz_target!( |data: &[u8]| {
    let mut map = fast_range_map::RangeMap::new();
    let mut map_sanity = rangemap::RangeMap::new();

    map.insert( 0..255, 1 );
    map_sanity.insert( 0..255, 1 );

    let mut sum = 0;
    for chunk in data.chunks_exact( 2 ) {
        let start = chunk[0] as u64;
        let length = std::cmp::max( 1, chunk[1] as u64 );
        let end = start + length;
        let range = start..end;
        for (range, _) in map.remove( range.clone() ) {
            sum += range.end - range.start;
        }
        map_sanity.remove( range.clone() );
    }

    let map: Vec< _ > = map.into_vec();
    let map_sanity: Vec< _ > = map_sanity.into_iter().collect();
    assert_eq!( map, map_sanity );
    assert_eq!( sum, 255 - map.iter().map( |(key, _)| key.end - key.start ).sum::< u64 >() );
});
