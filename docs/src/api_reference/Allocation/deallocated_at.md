## Allocation::deallocated_at

```rhai
fn deallocated_at(
    self: Allocation
) -> Option<Duration>
```

Returns when this allocation was freed, as a time offset from the start of the profiling.

### Examples

```rhai,%run
println((allocations().only_leaked())[0].deallocated_at());
println((allocations().only_temporary())[0].deallocated_at());
```
