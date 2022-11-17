## Graph::show_address_space

```rhai
fn show_address_space(
    self: Graph
) -> Graph
```

Configures the graph to show maps' used address space.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(maps())
    .show_address_space()
    .save();
```
