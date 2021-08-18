## AllocationList::only_last_size_larger_or_equal

```rhai
fn only_last_size_larger_or_equal(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that are part of an allocation chain where the last allocation's size is larger or equal to the given `threshold`.
