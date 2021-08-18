## Graph::trim_right

```rhai
fn trim_right(
    self: Graph
) -> Graph
```

Trims any empty space in the right portion of the graph.

### Examples

Here's a graph which has a significant amount of empty space at the end:

```rhai,%run,%hide-code
graph()
    .trim_left()
    .add(allocations().only_deallocated_until_at_most(data().runtime() * 0.75))
    .save();
```

By applying `trim_right` to it here's how it'll look like:

```rhai,%run,%hide-code
graph()
    .trim_left()
    .trim_right()
    .add(allocations().only_deallocated_until_at_most(data().runtime() * 0.75))
    .save();
```
