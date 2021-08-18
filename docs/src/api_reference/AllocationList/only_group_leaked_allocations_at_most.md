## AllocationList::only_group_leaked_allocations_at_most

```rhai
fn only_group_leaked_allocations_at_most(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that come from a stack trace which produced at most `threshold` leaked allocations.
