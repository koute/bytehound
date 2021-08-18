## println

```rhai
fn println()
```

```rhai
fn println(
    value: Any
)
```

```rhai
fn println(
    format: String,
    arg_1: Any
)
```

```rhai
fn println(
    format: String,
    arg_1: Any,
    arg_2: Any
)
```

```rhai
fn println(
    format: String,
    arg_1: Any,
    arg_2: Any,
    arg_3: Any
)
```

Prints out a given value or message, with optional Rust-like string interpolation.
(At the moment only `{}` is supported in the format string.)

For scripts executed through the scripting console it will print out the message
directly on the web page; for scripts executed through the `script` subcommand
it will print out the message on stdout.
