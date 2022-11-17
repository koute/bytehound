## Map::allocated_at

```rhai
fn allocated_at(
    self: Map
) -> Duration
```

Returns when this map was mapped, as a time offset from the start of the profiling.

### Examples

```rhai,%run
println(maps()[0].allocated_at());
```
