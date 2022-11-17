## Graph::show_rss

```rhai
fn show_rss(
    self: Graph
) -> Graph
```

Configures the graph to show maps' RSS.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add(maps())
    .show_rss()
    .save();
```
