## AllocationList::only_not_matching_deallocation_backtraces

```rhai
fn only_not_matching_deallocation_backtraces(
    self: AllocationList,
    backtrace_ids: [Backtrace|AllocationList|AllocationGroupList|Integer]
) -> AllocationList
```

```rhai
fn only_not_matching_deallocation_backtraces(
    self: AllocationList,
    backtrace_ids: Backtrace|AllocationList|AllocationGroupList|Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were not deallocated at one of the given `backtrace_ids`.
