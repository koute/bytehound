## Graph::only_non_empty_series

```rhai
fn only_non_empty_series(
    self: Graph
) -> Graph
```

Hides legend entries for any series which are empty.

### Examples

Let's say we have the following code:

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add("Leaked", allocations().only_leaked())
    .add("Temporary", allocations().only_temporary())
    .add("Remaining", allocations())
    .save();
```

As we can see we have an extra "Remaining" series which is empty; we can automatically hide it using `only_non_empty_series`:

```rhai,%run
graph()
    // %hide_next_line
    .trim()
    .add("Leaked", allocations().only_leaked())
    .add("Temporary", allocations().only_temporary())
    .add("Remaining", allocations())
    .only_non_empty_series()
    .save();
```
