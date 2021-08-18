## allocations

```rhai
fn allocations() -> AllocationList
```

Returns an allocation list of the the currently globally loaded data file; equivalent to `data().allocations()`.

If there is no globally loaded data file then it will throw an exception.
