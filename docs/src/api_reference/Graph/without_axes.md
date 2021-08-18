## Graph::without_axes

```rhai
fn without_axes(
    self: Graph
) -> Graph
```

Removes the horizonal and vertical axis of the graph.

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
    .without_axes()
    .save();
```
