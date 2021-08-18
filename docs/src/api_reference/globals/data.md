## data

```rhai
fn data() -> Data
```

Return the currently globally loaded data file.

When running the script through the scripting console this will return whatever data
you currently have loaded. When running through the `script` subcommand it will return
the data file specified with the `--data` parameter; if it wasn't specified then it
will throw an exception.