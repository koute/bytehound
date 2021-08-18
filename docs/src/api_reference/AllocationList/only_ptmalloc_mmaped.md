## AllocationList::only_ptmalloc_mmaped

```rhai
fn only_ptmalloc_mmaped(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only ptmalloc allocations which were internally allocated through `mmap`.
