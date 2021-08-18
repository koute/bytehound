## Graph::extend_until

```rhai
fn extend_until(
    self: Graph,
    duration: Duration
) -> Graph
```

Extends the graph until given `duration` as measured from the start of the profiling.

### Examples

Assuming we have the following graph:

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations())
    .save();
```

We can extend it to the right like this:

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations())
    .extend_until(data().runtime() + s(5))
    .save();
```
