## MapList::only_alive_at

```rhai
fn only_alive_at(
    self: MapList,
    durations: [Duration]
) -> MapList
```

Returns a new `MapList` with only the maps that were alive at all
of the times specified by `durations` as measured from the start of profiling.
