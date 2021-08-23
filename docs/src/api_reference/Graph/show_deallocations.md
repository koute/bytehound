## Graph::show_deallocations

```rhai
fn show_deallocations(
    self: Graph
) -> Graph
```

Configures the graph to show deallocations.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(allocations())
    .show_deallocations()
    .save();
```
