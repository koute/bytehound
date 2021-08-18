## AllocationList::len

```rhai
fn len(
    self: AllocationList
) -> Integer
```

Returns the number of allocations within the list.

### Examples

```rhai,%run
println(allocations().len());
```
