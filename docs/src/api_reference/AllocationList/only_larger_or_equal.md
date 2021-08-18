## AllocationList::only_larger_or_equal

```rhai
fn only_larger_or_equal(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose size is larger or equal to the given `threshold`.
