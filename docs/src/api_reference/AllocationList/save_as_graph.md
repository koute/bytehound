## AllocationList::save_as_graph

```rhai
fn save_as_graph(
    self: AllocationList
) -> AllocationList
```

```rhai
fn save_as_graph(
    self: AllocationList,
    path: String
) -> AllocationList
```

Saves the allocation list as a graph. The `path` argument is optional; if missing the filename will be automatically generated.

### Examples

```rhai,%run
allocations()
    .only_temporary()
    .save_as_graph("allocations.svg");
```
