## AllocationList::only_last_size_smaller

```rhai
fn only_last_size_smaller(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that are part of an allocation chain where the last allocation's size is smaller than the given `threshold`.
