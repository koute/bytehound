## Backtrace::strip

```rhai
fn strip(
    self: Backtrace
) -> Backtrace
```

Strips out useless junk from the backtrace.

### Examples

```rhai,%run
let groups = allocations().group_by_backtrace().sort_by_size();
let backtrace = groups[0][0].backtrace();

println("Before:");
println(backtrace);

println();
println("After:");
println(backtrace.strip());
```
