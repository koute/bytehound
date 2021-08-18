## chdir

```rhai
fn chdir(
    path: String
)
```

Changes the current directory to the given `path`.

Will physically change the current directory only for scripts executed through the `script` subcommand.
For scripts executed from the scripting console no access to the local filesystem is provided,
and a virtual filesystem will be simulated instead.
