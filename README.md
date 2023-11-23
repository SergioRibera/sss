# Generate basic binary Rust Application
> [!WARNING]
> This need [cargo-generate](https://github.com/cargo-generate/cargo-generate) to expand this template

# Requirements
- Rust
- Cargo
- [Cargo Generate](https://github.com/cargo-generate/cargo-generate)
- [Cargo Make](https://github.com/sagiegurari/cargo-make) (Optional)
- [Cargo Release](https://github.com/crate-ci/cargo-release) (Optional)

## Features
- Use `MakeFile.toml` to manage tasks
- Use `cargo-release` to make easy deploy new release
- Can you enable log by answer to `cargo-generate`
- github actions for ci/cd
    - Check format (using `cargo-fmt`)
    - Check quality code (using `cargo-clippy`)
- No extra deps
- MakeFile.toml optimized
- You can use the files to configurate your own checks
    - `rustfmt.toml` to check format
    - `clippy.toml` to check quality code

# Use this Template
```sh
cargo generate SrTemplates/Bevy
```

## CargoMake Tasks

* **check** - Check all issues, format and code quality
* **clean** - Clean all target directory
* **clippy** - Check code quality
* **default** - Check all issues, format and code quality
* **fix-all** - Try fix all clippy and format issues
* **fix-fmt** - Fix format
* **fmt** - Check format quality
* **test** - Check all unit test

## :bulb: Tips & tricks
If the template is used on a regular basis, [cargo-generate] allows to se
tup favorite templates and default variables.

To do this, open or create the file `$CARGO_HOME/cargo-generate.toml`, in
sert this:
```toml
[favorites.bbin]
git = "https://github.com/SrTemplates/BasicBin"
```

After this, the template can be expanded using `cargo generate bbin`.
