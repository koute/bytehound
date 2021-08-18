## AllocationList::only_ptmalloc_from_main_arena

```rhai
fn only_ptmalloc_from_main_arena(
    self: AllocationList
) -> AllocationList
```

Returns a new `AllocationList` with only ptmalloc allocations which were internally allocated on the main arena.
