## load

```rhai
fn load(
    path: String
) -> Data
```

Loads a new data file from the given path.

Makes sense only for scripts executed through the `script` subcommand.
Will throw an exception when called from the scripting console.
