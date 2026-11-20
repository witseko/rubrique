# Instructions for agents

Read this before changing Rubrique. The code is Rust (edition 2024) with
egui; `README.md` says what the app does and `CHANGELOG.md` what changed.

## Before a pull request

Run all four and fix what they report; CI runs the same with `--locked`:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo deny check
```

Add a line to the `Unreleased` section of `CHANGELOG.md` for anything a
user would notice (Added, Changed or Fixed), in the same voice as the
entries there: bold name, then what it does for the user.

Branch names are words joined by `-` only (`table-cell-edit-preview`), no
`claude/` prefix. Commit messages are one short imperative line.

## Releasing

A release is a commit on `main` plus a tag; no release branch or pull
request. Versions follow [Semantic Versioning](https://semver.org): pick
the bump from what the unreleased changelog entries hold.

- **Major** (2.0.0): a user has to change something on their side, such
  as a settings file that no longer loads without a migration, a removed
  feature or keybinding, a dropped OS version, or files saved differently.
- **Minor** (1.1.0): anything under Added, or a Changed entry that breaks
  nothing.
- **Patch** (1.0.1): Fixed entries only. One Added entry makes it a minor.

Group fixes from the same day into one patch release where you can.

1. In `CHANGELOG.md`, insert `## [X.Y.Z] - YYYY-MM-DD` under the
   `## [Unreleased]` heading (keep that heading, empty, above it), so the
   unreleased entries become the release notes.
2. Set `version = "X.Y.Z"` in `Cargo.toml` and in the `rubrique` package
   entry of `Cargo.lock` (or run `cargo check --locked` after editing
   `Cargo.toml` and commit the lock change it asks for).
3. Run the four checks above.
4. Commit as `release X.Y.Z`, tag it, and push both in one command so the
   workflows see the tag:

   ```sh
   git tag -a rubrique-vX.Y.Z -m "Rubrique X.Y.Z"
   git push origin main refs/tags/rubrique-vX.Y.Z
   ```

The `Release` workflow (`.github/workflows/release.yml`) builds the macOS
and Linux packages and publishes the GitHub release with the changelog
section for that version, then publishes a stable version to crates.io
(Trusted Publishing, no token; release candidates are not published there). It runs on the tag push, and also on any push
to `main` whose `Cargo.toml` version has no tag yet. If Actions did not
run (for example the account's spending limit was reached), start it by
hand from the Actions tab (`workflow_dispatch`) once that is fixed.

### Release candidates

Before a major or a large minor release, ship release candidates so
testers get the build first: `1.1.0-rc.1`, `1.1.0-rc.2`, and so on. Keep
the dot before the number (`rc.10` sorts after `rc.2`; `rc10` would not).

- Follow the same four steps with the full version (`1.1.0-rc.1`) in the
  changelog heading, `Cargo.toml`, the commit and the tag
  (`rubrique-v1.1.0-rc.1`).
- The workflow sees the hyphen and publishes a GitHub pre-release, which
  never becomes the "latest" release. The macOS app reports `1.1.0` in
  its bundle version (macOS accepts only numbers there); the file names
  and the About screen keep the suffix.
- After the first candidate, only fixes go in. Each fix ships as the next
  candidate, with its own changelog section.
- The stable release (`1.1.0`) is the last candidate with only the version
  changed. Its changelog section lists everything since the previous
  stable release: move the candidates' entries into it and remove their
  sections, so the release notes are complete.

Touch ID for encrypted scratches needs the signed app to embed a Developer
ID provisioning profile for `io.github.witseko.rubrique` (the data
protection keychain is open only to an authorized application identifier).
The workflow reads it from the `MACOS_PROVISIONING_PROFILE` secret (base64)
and passes its path to `bundle.sh` as `MACOS_PROFILE`, which adds the
`com.apple.application-identifier` and `com.apple.developer.team-identifier`
entitlements only then. A build without the profile still works; Touch ID
reports that it needs the app from a release.

## macOS disk image

`packaging/macos/bundle.sh` lays the `.dmg` out through Finder: window
size, icon size and the two icon positions, over
`packaging/macos/background.png`. That picture is drawn by a script:

```sh
swift packaging/macos/background.swift packaging/macos/background.png
```

The icon positions live in both `bundle.sh` and `background.swift`;
change them together and remake the picture.

## Linux app icon

`assets/icon/linux/` holds a larger copy of the app icon for Linux: the
tile fills 960 of 1024 px, where the macOS one (`assets/icon/`) keeps the
macOS margin and looks small in GNOME and Ubuntu docks. Change both SVGs
together, then remake the Linux PNGs (macOS only):

```sh
swift packaging/linux/icons.swift assets/icon/linux/rubrique.svg assets/icon/linux
```

## Screenshots

Build the release you are documenting. On a Mac with a 2x display,
run the capture scripts one at a time from a terminal with Screen
Recording access. The Compare and editor scripts also need Accessibility
access. Leave the keyboard and mouse idle while they drive the sample window.

```sh
cargo build --locked
docs/screenshots/tools/ai.sh target/debug/rubrique
docs/screenshots/tools/editor.sh target/debug/rubrique
docs/screenshots/tools/themes.sh target/debug/rubrique
```

The scripts use isolated settings and copy `docs/screenshots/sample/notes/`
into a temporary Git repository at `/tmp/field-notes`. Review the three
PNGs in `docs/screenshots/` before copying them to the website.

- `ai.png` shows the Compare screen for the survey handoff. The Claude Code
  and Codex stand-ins read the answers in `docs/screenshots/tools/ai/`.
  Keep two distinct edits so the screen shows both choices.
- `editor.png` shows Find highlighting `tide` in the rendered `survey.md`.
- `themes.png` shows the same guide with its Structure panel open, in all
  five built-in themes.

When updating rubrique.dev, use these same PNGs. Follow the website
README's screenshot instructions to regenerate its AVIF and WebP files.
