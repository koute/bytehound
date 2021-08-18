## AllocationList::only_deallocated_after_at_least

```rhai
fn only_deallocated_after_at_least(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were deallocated after at least `duration`
from the start of profiling.
