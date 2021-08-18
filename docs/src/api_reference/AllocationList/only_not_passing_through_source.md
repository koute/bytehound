## AllocationList::only_not_passing_through_source

```rhai
fn only_not_passing_through_source(
    self: AllocationList,
    regex: String
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose backtrace does **not** contain a frame which passes through a source file which matches a given regex.

The flavor of regexps used here is the same as Rust's [`regex` crate](https://docs.rs/regex).
