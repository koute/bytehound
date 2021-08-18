## Graph::trim

```rhai
fn trim_right(
    self: Graph
) -> Graph
```

Trims any empty space on both sides of the graph.

### Examples

Here's a graph which has a significant amount of empty on both sides:

```rhai,%run,%hide-code
let xs = allocations()
    .only_allocated_after_at_least(data().runtime() * 0.25)
    .only_deallocated_until_at_most(data().runtime() * 0.75);

graph()
    .add(xs)
    .save();
```

By applying `trim` to it here's how it'll look like:

```rhai,%run,%hide-code
let xs = allocations()
    .only_allocated_after_at_least(data().runtime() * 0.25)
    .only_deallocated_until_at_most(data().runtime() * 0.75);

graph()
    .trim()
    .add(xs)
    .save();
```
