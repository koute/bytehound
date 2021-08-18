## AllocationList::only_first_size_larger

```rhai
fn only_first_size_larger(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that are part of an allocation chain where the first allocation's size is larger than the given `threshold`.
