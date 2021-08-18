## AllocationList::only_backtrace_length_at_most

```rhai
fn only_backtrace_length_at_most(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that have a backtrace which has at most `threshold` many frames.
