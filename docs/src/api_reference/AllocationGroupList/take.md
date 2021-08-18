## AllocationGroupList::take

```rhai
fn take(
    self: AllocationGroupList,
    count: AllocationGroupList
) -> AllocanioGroupList
```

Returns a new list with at most `count` items.

### Examples

```rhai,%run
let groups = allocations().group_by_backtrace();
println(groups.len());
println(groups.take(3).len());
println(groups.take(100).len());
```
