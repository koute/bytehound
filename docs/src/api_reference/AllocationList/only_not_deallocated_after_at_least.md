## AllocationList::only_not_deallocated_after_at_least

```rhai
fn only_not_deallocated_after_at_least(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were **not** deallocated after at least `duration`
from the start of profiling.
