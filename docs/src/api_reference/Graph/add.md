## Graph::add

```rhai
fn add(
    self: Graph,
    list: AllocationList|AllocationGroupList|MapList
) -> Graph
```

```rhai
fn add(
    self: Graph,
    series_name: String,
    list: AllocationList|MapList
) -> Graph
```

Adds a new series to the graph with the given `list`.

If you add multiple allocation lists the graph will become an area graph, where every extra `add` will
only add new allocations to the graph which were not present in any of the previously added lists.

A single graph can either show allocations, or maps. Mixing both in a single graph is not supported.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add("Only leaked", allocations().only_leaked())
    .add("Remaining", allocations())
    .save();
```
