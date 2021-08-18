## AllocationList::only_chain_length_at_most

```rhai
fn only_chain_length_at_most(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose whole allocation chain was at most `threshold` allocations long.

For example, for the following allocation pattern:

```c
void * a0 = malloc(size);
void * a1 = realloc(a0, size + 1);

void * b0 = malloc(size);
void * b1 = realloc(b0, size + 1);
void * b2 = realloc(b1, size + 2);
```

this code:

```rhai
allocations().only_chain_length_at_most(2)
```

will only match `a0` and `a1`, since their whole allocation chain has at most two allocations.
