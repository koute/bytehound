## AllocationList::only_jemalloc

```rhai
fn only_jemalloc(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only allocations which were allocated through
one of the jemalloc interfaces.
