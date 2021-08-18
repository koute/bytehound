## AllocationList::only_address_at_most

```rhai
fn only_address_at_most(
    self: AllocationList,
    address: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose address is equal or lower than the one specified.
