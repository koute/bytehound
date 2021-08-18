## argv

```rhai
fn argv() -> [String]
```

Returns a list of arguments passed on the command-line.

Makes sense only for scripts executed through the `script` subcommand.
For scripts executed from the scripting console this will always return
an empty array.
