## AllocationList::only_chain_alive_for_at_most

```rhai
fn only_chain_alive_for_at_most(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose whole allocation chain was alive for at most the given `duration`.

This considers the whole span of time from when the allocation was first allocated (e.g. through `malloc`), through any potential reallocations,
and until it was freed (e.g. through `free`) or the profiling was stopped. It will match every allocation in that chain.

For example, for the following allocation pattern:

```c
void * a0 = malloc(size);
sleep(1);

void * a1 = realloc(a0, size + 1);
void * a2 = realloc(a2, size + 2);
free(a2);
```

this code:

```rhai
allocations().only_chain_alive_for_at_most(s(2))
```

will match all three allocations (`a0`, `a1`, `a2`), since their whole allocation chain lived for less than two seconds.

You can use [`only_alive_for_at_most`](./only_alive_for_at_most.md) if you'd like to only take
the lifetime of a single allocation into account.
