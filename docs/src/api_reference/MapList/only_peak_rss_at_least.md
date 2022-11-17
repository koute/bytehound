## MapList::only_peak_rss_at_least

```rhai
fn only_peak_rss_at_least(
    self: MapList,
    threshold: Integer
) -> MapList
```

Returns a new `MapList` with only those maps whose peak RSS is at least the given `threshold`.
