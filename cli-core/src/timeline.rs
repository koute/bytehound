use crate::{Data, OperationId};

pub fn build_timeline(
    data: &Data,
    timestamp_min: common::Timestamp,
    timestamp_max: common::Timestamp,
    ops: &[OperationId]
) -> Vec< TimelinePoint > {
    let granularity = std::cmp::max( (timestamp_max - timestamp_min).as_usecs() / 1000, 1 );
    let mut output = Vec::with_capacity( 1000 + 2 );

    let mut current_time: u64 = 0;
    let mut current_usage: i64 = 0;
    let mut current_max_usage: i64 = 0;
    let mut current_allocations: i64 = 0;
    let mut current_max_allocations: i64 = 0;
    let mut current_allocations_per_time: u64 = 0;
    let mut current_deallocations_per_time: u64 = 0;
    for op in ops {
        let timestamp;

        let mut next_usage = current_usage;
        let mut next_allocations = current_allocations;
        let allocation = data.get_allocation( op.id() );
        if op.is_allocation() {
            next_usage += allocation.size as i64;
            next_allocations += 1;
            timestamp = allocation.timestamp;
        } else if op.is_deallocation() {
            next_usage -= allocation.size as i64;
            next_allocations -= 1;
            timestamp = allocation.deallocation.as_ref().unwrap().timestamp;
        } else if op.is_reallocation() {
            let old_allocation = data.get_allocation( allocation.reallocated_from.unwrap() );
            next_usage += allocation.size as i64;
            next_usage -= old_allocation.size as i64;
            timestamp = allocation.timestamp;
        } else {
            unreachable!()
        }

        let next_time = timestamp.as_usecs() / granularity;
        if current_time == 0 {
            current_time = next_time;
        } else if current_time != next_time {
            // Since the allocations are gathered in parallel and are not guaranteed
            // to be strictly ordered we could - in theory - temporarily hit a negative memory usage.
            let memory_usage = std::cmp::max( 0, current_max_usage ) as u64;
            let allocations = std::cmp::max( 0, current_max_allocations ) as u64;
            while current_time < next_time {
                let point = TimelinePoint {
                    timestamp: current_time * granularity,
                    memory_usage,
                    allocations,
                    allocations_per_time: current_allocations_per_time,
                    deallocations_per_time: current_deallocations_per_time,
                };
                output.push( point );
                current_time += 1;
                current_allocations_per_time = 0;
                current_deallocations_per_time = 0;
            }
            current_max_usage = 0;
            current_max_allocations = 0;
        }

        current_usage = next_usage;
        current_allocations = next_allocations;
        current_max_usage = std::cmp::max( current_max_usage, next_usage );
        current_max_allocations = std::cmp::max( current_max_allocations, next_allocations );

        if op.is_deallocation() {
            current_deallocations_per_time += 1;
        } else {
            current_allocations_per_time += 1;
        }
    }

    output.push( TimelinePoint {
        timestamp: current_time * granularity,
        memory_usage: std::cmp::max( 0, current_max_usage ) as u64,
        allocations: std::cmp::max( 0, current_max_allocations ) as u64,
        allocations_per_time: current_allocations_per_time,
        deallocations_per_time: current_deallocations_per_time,
    });

    output.push( TimelinePoint {
        timestamp: current_time * granularity + 1,
        memory_usage: std::cmp::max( 0, current_usage ) as u64,
        allocations: std::cmp::max( 0, current_allocations ) as u64,
        allocations_per_time: 0,
        deallocations_per_time: 0,
    });

    output
}

pub struct TimelinePoint {
    pub timestamp: u64,
    pub memory_usage: u64,
    pub allocations: u64,
    pub allocations_per_time: u64,
    pub deallocations_per_time: u64
}
