## AllocationList::only_alive_for_at_least

```rhai
fn only_alive_for_at_least(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were alive for at least the given `duration`.

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
allocations().only_alive_for_at_least(s(1))
```

will only match the first `a0` allocation since only it lived for at least one second.

You can use [`only_chain_alive_for_at_least`](./only_chain_alive_for_at_least.md) if you'd like to take
the lifetime of the whole allocation chain into account starting from the very first `malloc` and
persisting through any future reallocations.
