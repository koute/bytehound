## AllocationList::only_alive_at

```rhai
fn only_alive_at(
    self: AllocationList,
    durations: [Duration]
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were alive at all
of the times specified by `durations` as measured from the start of profiling.
