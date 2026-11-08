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
request. Minor versions (0.x.0) are for new features, patch versions for
fixes only.

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
section for that version. It runs on the tag push, and also on any push
to `main` whose `Cargo.toml` version has no tag yet. If Actions did not
run (for example the account's spending limit was reached), start it by
hand from the Actions tab (`workflow_dispatch`) once that is fixed.

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

`docs/screenshots/themes.png` is remade by a script (macOS only):

```sh
cargo build && docs/screenshots/tools/themes.sh target/debug/rubrique
```

It opens `docs/screenshots/sample/notes/todo.md` in a 1100 x 700 window
once per theme, with its own settings folder and no tmux on the PATH,
captures each window, and joins a vertical strip of each capture into one
image with the theme name under it. Change the sample folder rather than
the script when the picture should show something else.

`docs/screenshots/ai.png` is a staged Compare run on the same sample
folder, remade by another script (macOS only):

```sh
cargo build && docs/screenshots/tools/ai.sh target/debug/rubrique
```

Run it from a terminal with Accessibility and Screen Recording access, and
leave the keyboard and mouse alone while it runs: it drives the window
with synthetic clicks and keystrokes (`tools/drive.swift`). The app gets
stand-in `claude`, `codex` and `ollama` programs that print the answers in
`tools/ai`, so no agent runs. The script asks "make it shorter" with
Compare set to Codex, captures the Ask bar with the hover summary of
`ideas.md`, opens the merge screen, captures it, and pastes the bar and
the card onto the merge screen with `tools/paste.swift` (the app does not
open the Ask bar over the merge screen). Change the answers in `tools/ai`
when the picture should show something else; keep them different from
each other in two places so the merge screen has two changes to decide.
