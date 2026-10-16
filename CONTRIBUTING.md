# Contributing

Bug reports and ideas are welcome as
[issues](https://github.com/naqoyqatsi83/salt-radar/issues/new/choose). For a
security problem, use the private report described in [SECURITY.md](SECURITY.md)
instead.

## Reporting a bug

Say which salt-radar version you run (the What's new page of the help, `?`,
shows it), the Salt version and OS of the master, and what you did, saw and
expected. A screenshot, or a popup's text copied with `y`, helps with anything
visual; remove minion names, hostnames and other details of your environment
you don't want public.

## Pull requests

Open an issue first for anything bigger than a small fix, so the change can be
agreed on before you write it. Pull requests go to the `dev` branch; `main`
only holds releases.

- Keep `cargo clippy --all-targets -- -D warnings` and `cargo test` passing;
  CI runs both on every pull request. The minimum Rust version is the
  `rust-version` in `Cargo.toml`.
- salt-radar is read-only towards Salt outside admin mode: it must not change
  keys, files or minions, or send jobs to minions, unless the user asked for it
  there.
- Commit messages use conventional prefixes (`feat:`, `fix:`, `docs:`,
  `refactor:`, `chore:`), lowercase, with no period at the end, and name the
  issue in the body (`Closes #N`).
- Add a line under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) for
  anything a user would notice, in the imperative ("Add", "Fix").

See [Development](README.md#development) in the README for building and the
project layout.
