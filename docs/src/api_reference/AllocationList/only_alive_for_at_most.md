## AllocationList::only_alive_for_at_most

```rhai
fn only_alive_for_at_most(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were alive for at most the given `duration`.

This only considers the span of time from when the allocation was last allocated (e.g. through `malloc` or `realloc`)
until it was freed or reallocated (e.g. through `free` or `realloc`), or until the profiling was stopped.

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
allocations().only_alive_for_at_most(s(0.5))
```

will only match the last two allocations (`a1` and `a2`) since only they were alive for at most half a second.

You can use [`only_chain_alive_for_at_most`](./only_chain_alive_for_at_most.md) if you'd like to take
the lifetime of the whole allocation chain into account starting from the very first `malloc` and
persisting through any future reallocations.
