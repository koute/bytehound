## MapList::only_larger

```rhai
fn only_larger(
    self: MapList,
    threshold: Integer
) -> MapList
```

Returns a new `MapList` with only the maps whose size (address space) is larger than the given `threshold`.
