## AllocationList::only_deallocated_until_at_most

```rhai
fn only_deallocated_until_at_most(
    self: AllocationList,
    duration: Duration
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations that were deallocated until at most `duration`
from the start of profiling.
