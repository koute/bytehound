## Graph::save

```rhai
fn save(
    self: Graph
) -> Graph
```

```rhai
fn save(
    self: Graph,
    path: String
) -> Graph
```

Saves the graph to a file. The `path` argument is optional; if missing the filename will be automatically generated.

### Examples

```rhai,%run
graph()
    .add(allocations())
    .save("allocations.svg");
```
