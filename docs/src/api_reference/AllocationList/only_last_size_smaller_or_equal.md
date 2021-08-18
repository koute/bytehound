## AllocationList::only_last_size_smaller_or_equal

```rhai
fn only_last_size_smaller_or_equal(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that are part of an allocation chain where the last allocation's size is smaller or equal to the given `threshold`.
