## AllocationGroupList::sort_by_count_descending

```rhai
fn sort_by_count_descending(
    self: AllocationGroupList
) -> AllocationGroupList
```

Sorts the groups by allocation count in a descending order.

### Examples

```rhai,%run
let groups = allocations().group_by_backtrace().sort_by_count_descending();
println(groups[0].len());
println(groups[1].len());
println(groups[groups.len() - 1].len());
```
