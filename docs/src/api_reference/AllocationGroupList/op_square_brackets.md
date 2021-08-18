## AllocationGroupList::[]

```rhai
fn [](
    index: Integer
) -> AllocationList
```

Returns a given [`AllocationList`](../AllocationList.md) from the list.

### Examples

```rhai,%run
let groups = allocations().group_by_backtrace().sort_by_count();
println(groups[0].len());
println(groups[1].len());
```
