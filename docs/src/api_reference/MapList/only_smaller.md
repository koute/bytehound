## MapList::only_smaller

```rhai
fn only_smaller(
    self: MapList,
    threshold: Integer
) -> MapList
```

Returns a new `MapList` with only the maps whose size (address space) is smaller than the given `threshold`.
