## AllocationList::only_first_size_smaller

```rhai
fn only_first_size_smaller(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that are part of an allocation chain where the first allocation's size is smaller than the given `threshold`.
