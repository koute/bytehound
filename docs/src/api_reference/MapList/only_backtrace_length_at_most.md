## MapList::only_backtrace_length_at_most

```rhai
fn only_backtrace_length_at_most(
    self: MapList,
    threshold: Integer
) -> MapList
```

Returns a new `MapList` with only the maps that have a backtrace which has at most `threshold` many frames.
