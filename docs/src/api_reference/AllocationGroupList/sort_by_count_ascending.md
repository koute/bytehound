## AllocationGroupList::sort_by_count_ascending

```rhai
fn sort_by_count_ascending(
    self: AllocationGroupList
) -> AllocationGroupList
```

Sorts the groups by allocation count in an ascending order.

### Examples

```rhai,%run
let groups = allocations().group_by_backtrace().sort_by_count_ascending();
println(groups[0].len());
println(groups[1].len());
println(groups[groups.len() - 1].len());
```
