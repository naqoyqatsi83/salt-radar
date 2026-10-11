# salt-radar — project conventions

## Deployment target

salt-radar runs directly on a **bare-metal server or full VM** running a SaltStack
master.  It is NOT deployed inside containers or Kubernetes pods.

- The SaltStack master is installed via **vendor packages** (e.g. `salt-master`
  from the distro repo, SaltStack's official repo, or EPEL — **never** pip).
- The Python helper agent uses Salt's own Python (`/opt/saltstack/salt/bin/python3`
  or the system Python with the distro-packaged `salt` package) and accesses
  the master's filesystem directly: PKI keys at `/etc/salt/pki/master/minions/`,
  the job cache at `/var/cache/salt/master/`, `/proc/`, `systemctl`, etc.
- The Rust TUI and the Python agent run as sibling processes on the same
  machine (the master), communicating via NDJSON over stdin/stdout.
- Because the agent has direct filesystem access, there is no need for
  `kubectl exec` or any Kubernetes-aware fallback strategy.

## Testing on Kubernetes (test environment only)

The development test environment runs the Salt master inside Docker Desktop
Kubernetes.  This is NOT the deployment target — it's only for development.

- The Rust binary is cross-compiled inside a Docker container (`rust:latest`),
  then copied to the master pod via `kubectl cp` (or piped through stdin).
- The Python agent script is also uploaded to the pod separately.
- **When the salt-master pod is replaced or redeployed, ALL minion pods must
  also be redeployed** so they re-register with the new master's PKI keys.
  Salt minions authenticate by the master's key fingerprint; a new pod has a
  new key, so minions get `authentication failed` until restarted.
- Build a fully static binary for use inside the pod with
  `rustup target add x86_64-unknown-linux-musl` then
  `cargo build --release --target x86_64-unknown-linux-musl` to avoid
  glibc version mismatches between the build container and the pod.

## Versioning & tags

- **Format**: `v<major>.<minor>.<patch>` for releases (e.g. `v0.5.0`)
- **Choosing the number** (decide it yourself when asked to tag or release —
  don't ask): follow [SemVer](https://semver.org/) read through
  [PrideVer](https://pridever.org/) (`PROUD.DEFAULT.SHAME`), based on what's
  queued under `[Unreleased]` in CHANGELOG.md:
  - **major** is reserved for the user — stay on `0.x.y` until they say
    otherwise; never bump it yourself, even for breaking changes
  - **minor** (`x`, DEFAULT): any `### Added`, `### Changed` or `### Removed`
    entry — a new feature or a change in behavior; resets patch to 0
  - **patch** (`y`, SHAME): only `### Fixed` entries — fixes for things that
    were broken
  - e.g. `0.7.6` + a queue with an Added entry → `0.8.0`; with only Fixed → `0.7.7`
- **Release candidates**: use `-rc<N>` suffix (e.g. `v0.5.1-rc1`, `v0.5.1-rc2`) when a build is needed for testing but the feature set isn't final
- Tags are pushed to GitHub: `git push origin <tagname>`
- The `main` branch always reflects the latest stable release
- The `dev` branch is the active development branch

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

- Group changes under `## [<version>] - <YYYY-MM-DD>` for each release
- `## [Unreleased]` section at the top for pending changes
- Within each release, use subheadings: `### Added`, `### Changed`, `### Fixed`, `### Removed`
- Write in imperative mood ("Add", "Fix", "Change", not "Added", "Fixes", "Changed")
- One bullet per logical change; group related changes under the same bullet

## Commit messages

- Use conventional commit prefixes: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`
- Lowercase after the prefix, no period at end
- Keep subject under 70 characters
- Reference relevant issue numbers in the body when applicable

## Code style

- No comments unless the WHY is non-obvious
- Prefer editing existing files over creating new ones
- No backwards-compatibility hacks or dead code
- Error handling at system boundaries only (user input, external APIs)

## Releases

1. Update `version` in `Cargo.toml` (building updates `Cargo.lock`; commit both)
2. Move `[Unreleased]` entries under a new dated `## [version]` heading in CHANGELOG.md
3. Commit with message: `chore: release v<version>`
4. Tag: `git tag v<version>`
5. Push commit and tag: `git push origin dev && git push origin v<version>`
6. Merge `dev` into `main` with `--no-ff`, push `main` (this closes issues referenced as `Closes #N`)

Pushing the tag runs the **Release** workflow (`.github/workflows/release.yml`):
it builds the static musl binary and publishes a GitHub release with a
`.deb`, an `.rpm` and a `.tar.gz`. The `.deb`/`.rpm` install the binary, the
agent (`/usr/share/salt-radar/salt_radar_agent.py`) and the bundled themes
(`/usr/share/salt-radar/themes/`); the `.tar.gz` holds the same files for a
manual install. There is no local packaging step — the
package metadata (version, control file, RPM spec) is generated in the
workflow from `Cargo.toml`. After a release, check that the workflow passed
and that all three assets contain the themes.

## Building

- Debug: `cargo build`
- Release: `cargo build --release`
- Static binary for the test pod: see "Testing on Kubernetes" above (musl target)
- Tests: `cargo test`; lints: `cargo clippy --all-targets -- -D warnings`.
  The **CI** workflow (`.github/workflows/ci.yml`) runs both on every push and
  pull request to `dev`/`main` — keep them passing.
- Minimum Rust version: 1.88 (`rust-version` in `Cargo.toml`, set by the
  locked dependencies)
