## mkdir_p

```rhai
fn mkdir_p(
    path: String
)
```

Creates a new directory and all of its parent components if they are missing.

Equivalent to Rust's `std::fs::create_dir_all` or `mkdir -p`.

Will physically create directories only for scripts executed through the `script` subcommand.
For scripts executed from the scripting console no access to the local filesystem is provided,
and a virtual filesystem will be simulated instead.
