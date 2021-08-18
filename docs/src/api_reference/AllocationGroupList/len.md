## AllocationGroupList::len

```rhai
fn len(
    self: AllocationGroupList
) -> Integer
```

Returns the number of allocation groups within the list.

### Examples

```rhai,%run
println(allocations().group_by_backtrace().len());
```
