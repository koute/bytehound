## Graph::show_live_allocations

```rhai
fn show_live_allocations(
    self: Graph
) -> Graph
```

Configures the graph to show the number of allocations which are alive.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(allocations())
    .show_live_allocations()
    .save();
```
