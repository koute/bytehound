## allocations

```rhai
fn maps() -> MapList
```

Returns a map list of the the currently globally loaded data file; equivalent to `data().maps()`.

If there is no globally loaded data file then it will throw an exception.
