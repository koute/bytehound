## AllocationList::only_group_interval_at_least

```rhai
fn only_group_interval_at_least(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that come from a stack trace which produced allocations spanning at least `duration`,
as measured from the very first allocation, to the very last allocation from the same location.
