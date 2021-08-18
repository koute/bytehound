## AllocationList::only_allocated_after_at_least

```rhai
fn only_allocated_after_at_least(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were allocated after at least `duration`
from the start of profiling.
