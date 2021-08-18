## Graph::add

```rhai
fn add(
    self: Graph,
    allocations: AllocationList|AllocationGroupList
) -> Graph
```

```rhai
fn add(
    self: Graph,
    series_name: String,
    allocations: AllocationList
) -> Graph
```

Adds a new series to the graph with the given `allocations`.

If you add multiple allocation lists the graph will become an area graph, where every extra `add` will
only add new allocations to the graph which were not present in any of the previously added lists.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add("Only leaked", allocations().only_leaked())
    .add("Remaining", allocations())
    .save();
```
