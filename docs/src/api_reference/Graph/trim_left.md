## Graph::trim_left

```rhai
fn trim_left(
    self: Graph
) -> Graph
```

Trims any empty space in the left portion of the graph.

### Examples

Here's a graph which has a significant amount of empty space at the start with no allocations:

```rhai,%run,%hide-code
graph()
    .trim_right()
    .add(allocations().only_allocated_after_at_least(data().runtime() * 0.25))
    .save();
```

By applying `trim_left` to it here's how it'll look like:

```rhai,%run,%hide-code
graph()
    .trim_right()
    .trim_left()
    .add(allocations().only_allocated_after_at_least(data().runtime() * 0.25))
    .save();
```
