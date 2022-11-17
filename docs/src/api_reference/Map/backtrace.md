## Map::backtrace

```rhai
fn backtrace(
    self: Map
) -> Option<Backtrace>
```

Returns the backtrace of this map.

### Examples

```rhai,%run
println(maps()[0].backtrace());
```
