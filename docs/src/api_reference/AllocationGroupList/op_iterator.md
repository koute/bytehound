## AllocationGroupList::(iterator)

`AllocationGroupList` can be iterated with a `for`.

### Examples

```rhai,%run
for group in allocations().group_by_backtrace().take(2) {
    println("Allocations in group: {}", group.len());
}
```
