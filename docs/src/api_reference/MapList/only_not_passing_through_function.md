## MapList::only_not_passing_through_function

```rhai
fn only_not_passing_through_function(
    self: MapList,
    regex: String
) -> MapList
```

Returns a new `MapList` with only the maps whose backtrace does **not** contain a function which matches a given regex.

The flavor of regexps used here is the same as Rust's [`regex` crate](https://docs.rs/regex).