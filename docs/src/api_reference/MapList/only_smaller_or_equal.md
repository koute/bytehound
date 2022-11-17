## MapList::only_smaller_or_equal

```rhai
fn only_smaller_or_equal(
    self: MapList,
    threshold: Integer
) -> MapList
```

Returns a new `MapList` with only the maps whose size (address space) is smaller or equal to the given `threshold`.
