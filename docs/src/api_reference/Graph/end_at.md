## Graph::end_at

```rhai
fn end_at(
    self: Graph,
    duration: Duration
) -> Graph
```

Truncate or extend the graph until given `duration` as measured from the start of the profiling.

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
    .end_at(data().runtime() - s(2))
    .save();
```

It can also be used to extend the graph:

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations())
    .end_at(data().runtime() + s(5))
    .save();
```
