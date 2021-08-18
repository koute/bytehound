## AllocationList::only_backtrace_length_at_least

```rhai
fn only_backtrace_length_at_least(
    self: AllocationList,
    threshold: Integer
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that have a backtrace which has at least `threshold` many frames.
