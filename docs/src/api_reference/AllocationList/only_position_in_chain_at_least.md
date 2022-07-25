## AllocationList::only_position_in_chain_at_least

```rhai
fn only_position_in_chain_at_least(
    self: AllocationList,
    position: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose position in their allocation chain is at least equal to `position`.
