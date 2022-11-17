## Graph::start_at

```rhai
fn start_at(
    self: Graph,
    duration: Duration
) -> Graph
```

Make the graph start after the given `duration` as measured from the start of the profiling.

### Examples

Assuming we have the following graph:

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations())
    .save();
```

We can make it start later like this:

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations())
    .start_at(s(2))
    .save();
```
