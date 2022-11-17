## MapList::only_matching_deallocation_backtraces

```rhai
fn only_matching_deallocation_backtraces(
    self: MapList,
    backtrace_ids: [Backtrace|AllocationList|MapList|AllocationGroupList|Integer]
) -> MapList
```

```rhai
fn only_matching_deallocation_backtraces(
    self: MapList,
    backtrace_ids: Backtrace|AllocationList|MapList|AllocationGroupList|Integer
) -> MapList
```

Returns a new `MapList` with only the maps that were unmapped at one of the given `backtrace_ids`.
