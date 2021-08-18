## AllocationList::only_smaller

```rhai
fn only_smaller(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose size is smaller than the given `threshold`.
