use std::io::{self, Read};
use std::collections::HashMap;
use common::event::Event;
use common::Timestamp;
use crate::reader::parse_events;

fn format_count( count: usize ) -> String {
    if count < 1000 {
        format!( "{}", count )
    } else if count < 1000 * 1000 {
        format!( "{}K", count / 1000 )
    } else {
        format!( "{}M", count / (1000 * 1000) )
    }
}

pub fn analyze_size( fp: impl Read + Send + 'static ) -> Result< (), io::Error > {
    let (_, event_stream) = parse_events( fp )?;

    const S_OTHER: usize = 0;
    const S_ALLOC: usize = 1;
    const S_REALLOC: usize = 2;
    const S_FREE: usize = 3;
    const S_BACKTRACE: usize = 4;
    const S_FILE: usize = 5;
    const S_MAX: usize = 6;

    const SIZE_TO_NAME: &[&str] = &[
        "Other",
        "Alloc",
        "Realloc",
        "Free",
        "Backtrace",
        "Files"
    ];

    #[derive(Default)]
    struct Stats {
        size: usize,
        count: usize
    }

    let mut stats = Vec::new();
    stats.resize_with( S_MAX, Stats::default );

    let mut allocation_buckets = Vec::new();
    allocation_buckets.resize( 10, 0 );

    fn elapsed_to_bucket( elapsed: Timestamp ) -> usize {
        let s = elapsed.as_secs();
        if s < 1 {
            0
        } else if s < 10 {
            1
        } else if s < 30 {
            2
        } else if s < 60 {
            3
        } else if s < 60 * 2 {
            4
        } else if s < 60 * 5 {
            5
        } else if s < 60 * 10 {
            6
        } else if s < 60 * 60 {
            7
        } else {
            8
        }
    }

    let mut allocations = HashMap::new();
    for event in event_stream {
        let event = match event {
            Ok( event ) => event,
            Err( _ ) => break
        };

        let size = common::speedy::Writable::< common::speedy::LittleEndian >::bytes_needed( &event ).unwrap();
        let kind = match event {
            | Event::Alloc { .. } => S_ALLOC,
            | Event::AllocEx { id, timestamp, .. } => {
                allocations.insert( id, timestamp );
                S_ALLOC
            },
            | Event::Realloc { .. }
            | Event::ReallocEx { .. } => S_REALLOC,
            | Event::Free { .. } => S_FREE,
            | Event::FreeEx { id, timestamp, .. } => {
                if let Some( allocated_timestamp ) = allocations.remove( &id ) {
                    let elapsed = timestamp - allocated_timestamp;
                    allocation_buckets[ elapsed_to_bucket( elapsed ) ] += 1;
                }
                S_FREE
            },
            | Event::Backtrace { .. }
            | Event::PartialBacktrace { .. }
            | Event::PartialBacktrace32 { .. }
            | Event::Backtrace32 { .. } => S_BACKTRACE,
            | Event::File { .. } => S_FILE,
            _ => S_OTHER
        };

        stats[ kind ].size += size;
        stats[ kind ].count += 1;
    }

    *allocation_buckets.last_mut().unwrap() += allocations.len();

    let mut stats: Vec< _ > = stats.into_iter().enumerate().collect();
    stats.sort_by_key( |(_, stats)| !stats.size );

    println!( "Total event sizes:" );
    for (index, stats) in stats {
        println!( "  {}: {}MB ({} events)", SIZE_TO_NAME[ index ], stats.size / (1024 * 1024), format_count( stats.count ) );
    }

    println!( "\nAllocation lifetime buckets:" );
    for (index, count) in allocation_buckets.into_iter().enumerate() {
        let label = match index {
            0 => "< 1s",
            1 => "< 10s",
            2 => "< 30s",
            3 => "< 1m",
            4 => "< 2m",
            5 => "< 5m",
            6 => "< 10m",
            7 => "< 1h",
            8 => ">= 1h",
            9 => "Leaked",
            _ => unreachable!()
        };

        println!( "  {}: {}", label, format_count( count ) );
    }

    Ok(())
}