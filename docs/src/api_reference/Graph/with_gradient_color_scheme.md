## Graph::with_gradient_color_scheme

```rhai
fn with_gradient_color_scheme(
    self: Graph,
    start: String,
    end: String
) -> Graph
```

Sets the graph color scheme so that the bottommost series is of the `start` color
and the topmost series is of the `end` color.

### Examples

```rhai,%run
let xs = allocations().only_temporary();
graph()
    // %hide_next_line
    .trim()
    .add(xs.only_alive_for_at_least(data().runtime() * 0.8))
    .add(xs.only_alive_for_at_least(data().runtime() * 0.7))
    .add(xs.only_alive_for_at_least(data().runtime() * 0.6))
    .add(xs.only_alive_for_at_least(data().runtime() * 0.5))
    .add(xs.only_alive_for_at_least(data().runtime() * 0.4))
    .add(xs.only_alive_for_at_least(data().runtime() * 0.3))
    .add(xs.only_alive_for_at_least(data().runtime() * 0.2))
    .add(xs.only_alive_for_at_least(data().runtime() * 0.1))
    .with_gradient_color_scheme("red", "blue")
    .save();
```
