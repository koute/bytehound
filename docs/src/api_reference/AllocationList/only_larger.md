## AllocationList::only_larger

```rhai
fn only_larger(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose size is larger than the given `threshold`.
