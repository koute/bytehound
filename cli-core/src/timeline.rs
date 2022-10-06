use crate::{Data, OperationId, Map, MapUsage, UsageDelta};
use crate::Timestamp;

#[derive(Copy, Clone, derive_more::Add, derive_more::Sub, Default, Debug)]
pub struct AllocationDelta {
    pub memory_usage: i64,
    pub allocations: i64,
}

impl< 'a > From< &'a MapUsage > for UsageDelta {
    fn from( usage: &'a MapUsage ) -> Self {
        UsageDelta {
            address_space: usage.address_space as i64,
            anonymous: usage.anonymous as i64,
            shared_clean: usage.shared_clean as i64,
            shared_dirty: usage.shared_dirty as i64,
            private_clean: usage.private_clean as i64,
            private_dirty: usage.private_dirty as i64,
            swap: usage.swap as i64
        }
    }
}

impl Map {
    pub fn emit_ops( &self, output: &mut Vec< (Timestamp, UsageDelta) > ) {
        let mut last = UsageDelta::default();
        for usage in &self.usage_history {
            let current: UsageDelta = usage.into();
            output.push( (usage.timestamp, current - last) );
            last = current;
        }
    }
}

pub trait Delta: Copy + Clone + Default + std::ops::Add< Output = Self > + std::ops::Sub< Output = Self > {
    fn max( lhs: Self, rhs: Self ) -> Self;
    fn min( lhs: Self, rhs: Self ) -> Self;

    fn at_least_zero( self ) -> Self {
        Self::max( self, Self::default() )
    }
    fn at_most_zero( self ) -> Self {
        Self::min( self, Self::default() )
    }
}

impl Delta for i64 {
    fn max( lhs: Self, rhs: Self ) -> Self {
        std::cmp::max( lhs, rhs )
    }

    fn min( lhs: Self, rhs: Self ) -> Self {
        std::cmp::min( lhs, rhs )
    }
}

impl Delta for AllocationDelta {
    fn max( lhs: Self, rhs: Self ) -> Self {
        AllocationDelta {
            memory_usage: std::cmp::max( lhs.memory_usage, rhs.memory_usage ),
            allocations: std::cmp::max( lhs.allocations, rhs.allocations )
        }
    }

    fn min( lhs: Self, rhs: Self ) -> Self {
        AllocationDelta {
            memory_usage: std::cmp::min( lhs.memory_usage, rhs.memory_usage ),
            allocations: std::cmp::min( lhs.allocations, rhs.allocations )
        }
    }
}

impl Delta for UsageDelta {
    fn max( lhs: Self, rhs: Self ) -> Self {
        UsageDelta {
            address_space: std::cmp::max( lhs.address_space, rhs.address_space ),
            anonymous: std::cmp::max( lhs.anonymous, rhs.anonymous ),
            shared_clean: std::cmp::max( lhs.shared_clean, rhs.shared_clean ),
            shared_dirty: std::cmp::max( lhs.shared_dirty, rhs.shared_dirty ),
            private_clean: std::cmp::max( lhs.private_clean, rhs.private_clean ),
            private_dirty: std::cmp::max( lhs.private_dirty, rhs.private_dirty ),
            swap: std::cmp::max( lhs.swap, rhs.swap ),
        }
    }

    fn min( lhs: Self, rhs: Self ) -> Self {
        UsageDelta {
            address_space: std::cmp::min( lhs.address_space, rhs.address_space ),
            anonymous: std::cmp::min( lhs.anonymous, rhs.anonymous ),
            shared_clean: std::cmp::min( lhs.shared_clean, rhs.shared_clean ),
            shared_dirty: std::cmp::min( lhs.shared_dirty, rhs.shared_dirty ),
            private_clean: std::cmp::min( lhs.private_clean, rhs.private_clean ),
            private_dirty: std::cmp::min( lhs.private_dirty, rhs.private_dirty ),
            swap: std::cmp::min( lhs.swap, rhs.swap ),
        }
    }
}

pub fn build_allocation_timeline(
    data: &Data,
    timestamp_min: common::Timestamp,
    timestamp_max: common::Timestamp,
    ops: &[OperationId]
) -> Vec< TimelinePoint< AllocationDelta > > {
    build_timeline(
        timestamp_min,
        timestamp_max,
        1000,
        ops.iter().map( |op| {
            let allocation = data.get_allocation( op.id() );
            if op.is_allocation() {
                let delta = AllocationDelta {
                    memory_usage: allocation.size as i64,
                    allocations: 1
                };
                (allocation.timestamp, delta)
            } else if op.is_deallocation() {
                let delta = AllocationDelta {
                    memory_usage: -(allocation.size as i64),
                    allocations: -1
                };
                (allocation.deallocation.as_ref().unwrap().timestamp, delta)
            } else if op.is_reallocation() {
                let old_allocation = data.get_allocation( allocation.reallocated_from.unwrap() );
                let delta = AllocationDelta {
                    memory_usage: allocation.size as i64 - old_allocation.size as i64,
                    allocations: 0
                };
                (allocation.timestamp, delta)
            } else {
                unreachable!()
            }
        })
    )
}

pub fn build_map_timeline(
    timestamp_min: common::Timestamp,
    timestamp_max: common::Timestamp,
    ops: &[(Timestamp, UsageDelta)]
) -> Vec< TimelinePoint< UsageDelta > > {
    build_timeline(
        timestamp_min,
        timestamp_max,
        1000,
        ops.iter().copied()
    )
}

fn build_timeline< T >(
    timestamp_min: common::Timestamp,
    timestamp_max: common::Timestamp,
    point_count: usize,
    ops: impl Iterator< Item = (Timestamp, T) >
) -> Vec< TimelinePoint< T > > where T: Delta {
    let granularity = std::cmp::max( (timestamp_max - timestamp_min).as_usecs() / point_count as u64, 1 );
    let mut output = Vec::with_capacity( point_count + 2 );

    let mut current_time: u64 = 0;
    let mut current = T::default();
    let mut current_max = T::default();
    let mut current_positive_per_time = T::default();
    let mut current_negative_per_time = T::default();
    for (timestamp, delta) in ops {
        let mut next = current;

        next = next + delta;

        let next_time = timestamp.as_usecs() / granularity;
        if current_time == 0 {
            current_time = next_time;
        } else if current_time != next_time {
            while current_time < next_time {
                let point = TimelinePoint {
                    timestamp: current_time * granularity,
                    // Since the allocations are gathered in parallel and are not guaranteed
                    // to be strictly ordered we could - in theory - temporarily hit a negative memory usage.
                    value: current_max.at_least_zero(),
                    positive_change: current_positive_per_time.at_least_zero(),
                    negative_change: current_negative_per_time.at_least_zero(),
                };
                output.push( point );
                current_time += 1;
                current_positive_per_time = Default::default();
                current_negative_per_time = Default::default();
            }
            current_max = Default::default();
        }

        current = next;
        current_max = T::max( current_max, next );
        current_positive_per_time = current_positive_per_time + delta.at_least_zero();
        current_negative_per_time = current_negative_per_time - delta.at_most_zero();
    }

    if output.is_empty() {
        output.push( TimelinePoint {
            timestamp: current_time * granularity.saturating_sub( 1 ),
            value: Default::default(),
            positive_change: Default::default(),
            negative_change: Default::default(),
        });
    }

    output.push( TimelinePoint {
        timestamp: current_time * granularity,
        value: current_max.at_least_zero(),
        positive_change: current_positive_per_time.at_least_zero(),
        negative_change: current_negative_per_time.at_least_zero(),
    });

    output.push( TimelinePoint {
        timestamp: current_time * granularity + 1,
        value: current.at_least_zero(),
        positive_change: Default::default(),
        negative_change: Default::default(),
    });

    output
}

#[derive(PartialEq, Eq, Debug)]
pub struct TimelinePoint< T > {
    pub timestamp: u64,
    pub value: T,
    pub positive_change: T,
    pub negative_change: T,
}

impl< T > std::ops::Deref for TimelinePoint< T > {
    type Target = T;
    fn deref( &self ) -> &Self::Target {
        &self.value
    }
}

#[test]
fn test_build_timeline_oversampled() {
    let output = build_timeline::< i64 >(
        Timestamp::from_usecs( 0 ),
        Timestamp::from_usecs( 16 ),
        100,
        vec![
            (Timestamp::from_usecs( 4 ), 100),
            (Timestamp::from_usecs( 8 ), -25),
            (Timestamp::from_usecs( 12 ), 200)
        ].into_iter()
    );

    assert_eq!( output, vec![
        TimelinePoint {
            timestamp: 4,
            value: 100,
            positive_change: 100,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 5,
            value: 100,
            positive_change: 0,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 6,
            value: 100,
            positive_change: 0,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 7,
            value: 100,
            positive_change: 0,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 8,
            value: 75,
            positive_change: 0,
            negative_change: 25,
        },
        TimelinePoint {
            timestamp: 9,
            value: 75,
            positive_change: 0,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 10,
            value: 75,
            positive_change: 0,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 11,
            value: 75,
            positive_change: 0,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 12,
            value: 275,
            positive_change: 200,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 13,
            value: 275,
            positive_change: 0,
            negative_change: 0,
        },
    ]);
}

#[test]
fn test_build_timeline_undersampled() {
    let output = build_timeline::< i64 >(
        Timestamp::from_usecs( 4 ),
        Timestamp::from_usecs( 28 ),
        3,
        vec![
            (Timestamp::from_usecs( 4 ), 4), // 4
            (Timestamp::from_usecs( 5 ), 1), // 5
            (Timestamp::from_usecs( 6 ), 1), // 6
            (Timestamp::from_usecs( 7 ), 1), // 7
            (Timestamp::from_usecs( 8 ), 1), // 8
            (Timestamp::from_usecs( 9 ), 1), // 9
            (Timestamp::from_usecs( 10 ), 1), // 10
            (Timestamp::from_usecs( 11 ), 1), // 11
            (Timestamp::from_usecs( 12 ), 1), // 12
            (Timestamp::from_usecs( 13 ), -1), // 11
            (Timestamp::from_usecs( 14 ), -1), // 10
            (Timestamp::from_usecs( 15 ), -1), // 9
            (Timestamp::from_usecs( 16 ), -1), // 8
            (Timestamp::from_usecs( 17 ), -1), // 7
            (Timestamp::from_usecs( 18 ), -1), // 6
            (Timestamp::from_usecs( 19 ), -1), // 5
            (Timestamp::from_usecs( 20 ), -1), // 4
            (Timestamp::from_usecs( 21 ), 10), // 14
            (Timestamp::from_usecs( 22 ), 10), // 24
            (Timestamp::from_usecs( 23 ), 10), // 34
            (Timestamp::from_usecs( 24 ), 10), // 44
            (Timestamp::from_usecs( 25 ), 10), // 54
            (Timestamp::from_usecs( 26 ), 10), // 64
            (Timestamp::from_usecs( 27 ), 10), // 74
            (Timestamp::from_usecs( 28 ), 10), // 84
        ].into_iter()
    );

    assert_eq!( output, vec![
        TimelinePoint {
            timestamp: 8,
            value: 12,
            positive_change: 12,
            negative_change: 3,
        },
        TimelinePoint {
            timestamp: 16,
            value: 34,
            positive_change: 30,
            negative_change: 5,
        },
        TimelinePoint {
            timestamp: 24,
            value: 84,
            positive_change: 50,
            negative_change: 0,
        },
        TimelinePoint {
            timestamp: 25,
            value: 84,
            positive_change: 0,
            negative_change: 0,
        },
    ]);
}
