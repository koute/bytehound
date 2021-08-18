## AllocationList::only_chain_length_at_least

```rhai
fn only_chain_length_at_least(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose whole allocation chain was at least `threshold` allocations long.

For example, for the following allocation pattern:

```c
void * a0 = malloc(size);

void * b0 = malloc(size);
void * b1 = realloc(b0, size + 1);
```

this code:

```rhai
allocations().only_chain_length_at_least(2)
```

will only match `b0` and `b1`, since their whole allocation chain has at least two allocations.
