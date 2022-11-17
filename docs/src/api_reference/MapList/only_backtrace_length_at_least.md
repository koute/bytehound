## MapList::only_backtrace_length_at_least

```rhai
fn only_backtrace_length_at_least(
    self: MapList,
    threshold: Integer
) -> MapList
```

Returns a new `MapList` with only the maps that have a backtrace which has at least `threshold` many frames.
