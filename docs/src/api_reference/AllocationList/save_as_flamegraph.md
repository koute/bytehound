## AllocationList::save_as_flamegraph

```rhai
fn save_as_flamegraph(
    self: AllocationList
) -> AllocationList
```

```rhai
fn save_as_flamegraph(
    self: AllocationList,
    path: String
) -> AllocationList
```

Saves the allocation list as a flamegraph. The `path` argument is optional; if missing the filename will be automatically generated.

### Examples

```rhai,%run
allocations()
    .only_temporary()
    .save_as_flamegraph("allocations.svg");
```
