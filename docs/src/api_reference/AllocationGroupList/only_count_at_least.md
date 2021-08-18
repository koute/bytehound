## AllocationGroupList::only_count_at_least

```rhai
fn only_count_at_least(
    self: AllocationGroupList,
    threshold: Integer
) -> AllocationGroupList
```

Returns a new `AllocationGroupList` with only those groups where the number of allocations is
at least `threshold` allocations or more.
