## MapList::only_passing_through_function

```rhai
fn only_passing_through_function(
    self: MapList,
    regex: String
) -> MapList
```

Returns a new `MapList` with only the maps whose backtrace contains a function which matches a given regex.

The flavor of regexps used here is the same as Rust's [`regex` crate](https://docs.rs/regex).
