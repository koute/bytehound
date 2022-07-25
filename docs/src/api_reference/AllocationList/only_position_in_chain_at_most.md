## AllocationList::only_position_in_chain_at_most

```rhai
fn only_position_in_chain_at_most(
    self: AllocationList,
    position: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose position in their allocation chain is at most equal to `position`.
