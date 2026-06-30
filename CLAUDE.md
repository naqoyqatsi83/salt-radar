# salt-radar — project conventions

## Versioning & tags

- **Format**: `v<major>.<minor>.<patch>` for releases (e.g. `v0.5.0`)
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

1. Update version in `Cargo.toml` and `DEBIAN/control`
2. Move `[Unreleased]` entries under a new dated `## [version]` heading in CHANGELOG.md
3. Commit with message: `chore: release v<version>`
4. Tag: `git tag v<version>`
5. Push commit and tag: `git push origin dev && git push origin v<version>`
6. Merge `dev` into `main` with `--no-ff`, push `main`

## Building

- Debug: `cargo build`
- Release: `cargo build --release`
- Debian packaging from `/tmp/salt-radar-deb/` (copy binary, call dpkg-deb)
