## AllocationList::only_chain_leaked

```rhai
fn only_chain_leaked(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only those allocations where
their last allocation in their realloc chain was leaked.

A leaked allocation is an allocation which was never deallocated.
