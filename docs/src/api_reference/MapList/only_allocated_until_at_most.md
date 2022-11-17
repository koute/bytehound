## MapList::only_allocated_until_at_most

```rhai
fn only_allocated_until_at_most(
    self: MapList,
    duration: Duration
) -> MapList
```

Returns a new `MapList` with only the maps that were mapped until at most `duration`
from the start of profiling.
