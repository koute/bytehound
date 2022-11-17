## Map::deallocated_at

```rhai
fn deallocated_at(
    self: Map
) -> Option<Duration>
```

Returns when this map was unmapped, as a time offset from the start of the profiling.

### Examples

```rhai,%run
println((maps().only_leaked())[0].deallocated_at());
println((maps().only_temporary())[0].deallocated_at());
```
