## Graph::show_memory_usage

```rhai
fn show_memory_usage(
    self: Graph
) -> Graph
```

Configures the graph to show memory usage.

This is the default.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(allocations())
    .show_memory_usage()
    .save();
```
