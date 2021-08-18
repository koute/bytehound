## AllocationList::only_smaller_or_equal

```rhai
fn only_smaller_or_equal(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose size is smaller or equal to the given `threshold`.
