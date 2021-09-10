## AllocationList::only_group_max_total_usage_first_seen_at_most

```rhai
fn only_group_max_total_usage_first_seen_at_most(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that come from
a stack trace whose total maximum memory usage first peaked before at most
`duration` from the start of profiling.
