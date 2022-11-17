## MapList::only_peak_rss_at_most

```rhai
fn only_peak_rss_at_most(
    self: MapList,
    threshold: Integer
) -> MapList
```

Returns a new `MapList` with only those maps whose peak RSS is at most the given `threshold`.
