## AllocationList::&

```rhai
fn &(
    lhs: AllocationList,
    rhs: AllocationList
) -> AllocationList
```

Returns a new allocation list with all of the allocations that are both in `lhs` and `rhs`.

### Examples

Here are graphs of two distinct allocation lists:

```rhai,%run,%hide-code
graph()
    // %hide_next_line
    .trim_left()
    .add(allocations().only_temporary().only_deallocated_until_at_most(data().runtime() * 0.6))
    .save();
```

```rhai,%run,%hide-code
graph()
    .add(allocations().only_temporary().only_allocated_after_at_least(data().runtime() * 0.4))
    .save();
```

And here's how they look when merged through the `&` operator:

```rhai,%run
let lhs = allocations()
    .only_temporary()
    .only_deallocated_until_at_most(data().runtime() * 0.6);

let rhs = allocations()
    .only_temporary()
    .only_allocated_after_at_least(data().runtime() * 0.4);

graph()
    .add(lhs & rhs)
    .save();
```
