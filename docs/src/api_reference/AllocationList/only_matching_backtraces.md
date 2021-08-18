## AllocationList::only_matching_backtraces

```rhai
fn only_matching_backtraces(
    self: AllocationList,
    backtrace_ids: [Backtrace|AllocationList|AllocationGroupList|Integer]
) -> AllocationList
```

```rhai
fn only_matching_backtraces(
    self: AllocationList,
    backtrace_ids: Backtrace|AllocationList|AllocationGroupList|Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that come from one of the given `backtrace_ids`.
