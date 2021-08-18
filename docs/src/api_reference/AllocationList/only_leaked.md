## AllocationList::only_leaked

```rhai
fn only_leaked(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only leaked allocations.

A leaked allocation is an allocation which was never deallocated.

Opposite of [`only_temporary`](./only_temporary.md).
