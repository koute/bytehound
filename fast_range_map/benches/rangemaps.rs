use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::ops::Range;

fn generate_random() -> Vec< (Range<u64>, u64) > {
    let mut output = Vec::new();
    let mut rng = oorandom::Rand64::new( 1234567 );
    for n in 0..4096 {
        let address = rng.rand_range( 0..u32::MAX as u64 / 2 ) & !(4096 - 1);
        let length = rng.rand_range( 1..1024 ) * 4096;
        output.push( (address..address + length, n) );
    }
    output
}

fn generate_sequential() -> Vec< (Range<u64>, u64) > {
    let mut output = Vec::new();
    let mut rng = oorandom::Rand64::new( 1234567 );
    let mut address = 0;
    for n in 0..4096 {
        let length = rng.rand_range( 1..1024 ) * 4096;
        output.push( (address..address + length, n) );
        address += length;
    }
    output
}

fn generate_in_the_middle() -> Vec< (Range<u64>, u64) > {
    let mut output = Vec::new();
    let mut l = 0;
    let mut r = u32::MAX as u64;
    let mut odd_even = true;
    for n in 0..4096 {
        let address;
        let length = 1;

        if odd_even {
            address = l;
            l += 1;
        } else {
            address = r;
            r -= 1;
        }
        odd_even = !odd_even;
        output.push( (address..address + length, n) );
    }
    output
}

fn bench_rangemap( input: &[(Range<u64>, u64)] ) -> u64 {
    let mut c = 0;
    let mut map = rangemap::RangeMap::new();
    for (range, value) in input {
        map.insert( range.clone(), *value );
        c += 1;
    }

    c as u64
}

fn bench_btree_range_map( input: &[(Range<u64>, u64)] ) -> u64 {
    let mut c = 0;
    let mut map = btree_range_map::RangeMap::new();
    for (range, value) in input {
        map.insert( range.clone(), *value );
        c += 1;
    }

    c as u64
}

fn bench_fast_range_map( input: &[(Range<u64>, u64)] ) -> u64 {
    let mut c = 0;
    let mut map = fast_range_map::RangeMap::new();
    for (range, value) in input {
        map.insert( range.clone(), *value );
        c += 1;
    }

    c as u64
}

#[inline(never)]
fn run_benches( c: &mut Criterion, name: &str, run: fn( &[(Range<u64>, u64)] ) -> u64 ) {
    let input_random = generate_random();
    let input_sequential = generate_sequential();
    let input_in_the_middle = generate_in_the_middle();

    c.bench_function( &format!( "{} (random)", name ), |b| b.iter( || run( black_box( &input_random ) ) ) );
    c.bench_function( &format!( "{} (sequential)", name ), |b| b.iter( || run( black_box( &input_sequential ) ) ) );
    c.bench_function( &format!( "{} (in the middle)", name ), |b| b.iter( || run( black_box( &input_in_the_middle ) ) ) );
}

fn criterion_benchmark( c: &mut Criterion ) {
    run_benches( c, "rangemap", bench_rangemap );
    run_benches( c, "btree_range_map", bench_btree_range_map );
    run_benches( c, "fast_range_map", bench_fast_range_map );
}

criterion_group!( benches, criterion_benchmark );
criterion_main!( benches );
