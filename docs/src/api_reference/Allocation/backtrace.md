## Allocation::backtrace

```rhai
fn backtrace(
    self: Allocation
) -> Backtrace
```

Returns the backtrace of this allocation.

### Examples

```rhai,%run
println(allocations()[0].backtrace());
```
