## MapList::only_deallocated_after_at_least

```rhai
fn only_deallocated_after_at_least(
    self: MapList,
    duration: Duration
) -> MapList
```

Returns a new `MapList` with only the maps that were unmapped after at least `duration`
from the start of profiling.
