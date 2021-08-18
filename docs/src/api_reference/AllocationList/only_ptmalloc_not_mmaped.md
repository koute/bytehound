## AllocationList::only_ptmalloc_not_mmaped

```rhai
fn only_ptmalloc_not_mmaped(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only ptmalloc allocations which were internally not allocated through `mmap`.
