## MapList::only_deallocated_until_at_most

```rhai
fn only_deallocated_until_at_most(
    self: MapList,
    duration: Duration
) -> MapList
```

Returns a new `MapList` with only the maps that were unmapped until at most `duration`
from the start of profiling.
