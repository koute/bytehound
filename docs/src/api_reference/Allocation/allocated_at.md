## Allocation::allocated_at

```rhai
fn allocated_at(
    self: Allocation
) -> Duration
```

Returns when this allocation was made, as a time offset from the start of the profiling.

### Examples

```rhai,%run
println(allocations()[0].allocated_at());
```
