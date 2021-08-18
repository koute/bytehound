## Graph::without_legend

```rhai
fn without_legend(
    self: Graph
) -> Graph
```

Removes the legend from the graph.

### Examples

Before:

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add("Allocations", allocations())
    .save();
```

After:

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add("Allocations", allocations())
    .without_legend()
    .save();
```
