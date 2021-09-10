## AllocationList::only_not_jemalloc

```rhai
fn only_not_jemalloc(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only allocations which were *not* allocated through
one of the jemalloc interfaces.
