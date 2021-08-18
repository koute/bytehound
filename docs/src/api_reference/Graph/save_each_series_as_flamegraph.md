## Graph::save_each_series_as_flamegraph

```rhai
fn save_each_series_as_flamegraph(
    self: Graph
) -> Graph
```

```rhai
fn save_each_series_as_flamegraph(
    self: Graph,
    output_directory: String
) -> Graph
```

Saves each series of the graph into a separate file as a flamegraph. The `output_directory` argument is optional;
if missing the files will be generated in the current directory.

### Examples

```rhai,%run
graph()
    .add("Temporary", allocations().only_temporary())
    .add("Leaked", allocations().only_leaked())
    .save_each_series_as_flamegraph();
```
