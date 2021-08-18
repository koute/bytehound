## Graph::without_grid

```rhai
fn without_grid(
    self: Graph
) -> Graph
```

Removes the grid from the graph.

### Examples

Before:

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(allocations())
    .save();
```

After:

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(allocations())
    .without_grid()
    .save();
```
