## Graph::truncate_until

```rhai
fn truncate_until(
    self: Graph,
    duration: Duration
) -> Graph
```

Truncate the graph until given `duration` as measured from the start of the profiling.

### Examples

Assuming we have the following graph:

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations())
    .save();
```

We can truncate it like this:

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations())
    .truncate_until(data().runtime() - s(2))
    .save();
```
