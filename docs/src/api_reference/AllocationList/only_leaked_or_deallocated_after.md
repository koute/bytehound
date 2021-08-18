## AllocationList::only_leaked_or_deallocated_after

```rhai
fn only_leaked_or_deallocated_after(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were either leaked or deallocated
after `duration` from the start of profiling.
