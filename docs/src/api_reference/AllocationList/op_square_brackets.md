## AllocationList::[]

```rhai
fn [](
    index: Integer
) -> Allocation
```

Returns a given [`Allocation`](../Allocation.md) from the list.

### Examples

```rhai,%run
println(allocations()[0].backtrace());
```
