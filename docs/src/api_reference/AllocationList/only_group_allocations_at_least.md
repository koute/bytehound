## AllocationList::only_group_allocations_at_least

```rhai
fn only_group_allocations_at_least(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that come from a stack trace which produced at least `threshold` allocations.
