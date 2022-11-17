## AllocationList::only_from_maps

```rhai
fn only_from_maps(
    self: MapList,
    map_ids: Map|MapList|Integer
) -> MapList
```

```rhai
fn only_from_maps(
    self: MapList,
    map_ids: [Map|MapList|Integer]
) -> MapList
```

Returns a new `AllocationList` with only the allocations that come from within given `map_ids`.
