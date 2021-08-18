## AllocationList::only_passing_through_function

```rhai
fn only_passing_through_function(
    self: AllocationList,
    regex: String
) -> AllocationList
```

Returns a new `AllocationList` with only the allocations whose backtrace contains a function which matches a given regex.

The flavor of regexps used here is the same as Rust's [`regex` crate](https://docs.rs/regex).
