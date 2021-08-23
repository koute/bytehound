## Graph::show_new_allocations

```rhai
fn show_new_allocations(
    self: Graph
) -> Graph
```

Configures the graph to show new allocations.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(allocations())
    .show_new_allocations()
    .save();
```
