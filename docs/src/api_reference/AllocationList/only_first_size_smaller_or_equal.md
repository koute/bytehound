## AllocationList::only_first_size_smaller_or_equal

```rhai
fn only_first_size_smaller_or_equal(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that are part of an allocation chain where the first allocation's size is smaller or equal to the given `threshold`.
