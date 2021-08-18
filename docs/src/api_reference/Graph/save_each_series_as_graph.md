## Graph::save_each_series_as_graph

```rhai
fn save_each_series_as_graph(
    self: Graph
) -> Graph
```

```rhai
fn save_each_series_as_graph(
    self: Graph,
    output_directory: String
) -> Graph
```

Saves each series of the graph into a separate file. The `output_directory` argument is optional;
if missing the files will be generated in the current directory.

### Examples

```rhai,%run
graph()
    // %hide_next_line
    .trim_left()
    .add("Temporary", allocations().only_temporary())
    .add("Leaked", allocations().only_leaked())
    .save_each_series_as_graph();
```
