## AllocationList::only_temporary

```rhai
fn only_temporary(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only temporary allocations.

A temporary allocation is an allocation which was eventually deallocated.

Opposite of [`only_leaked`](./only_leaked.md).
