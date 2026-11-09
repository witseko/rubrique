# <img src="assets/icon/rubrique-128.png" alt="" width="40" align="top"> Rubrique

A fast editor, agentic on demand. [rubrique.dev](https://rubrique.dev)

Write Markdown (or plain text, JSON, SQL, XML) in a small native editor.
Press `Cmd/Ctrl+K` to hand the text to an agent, then accept or reject the
change. Uses the agents already on your computer: Claude Code, Codex, Grok or
a local model. No API keys. macOS and Linux.

<img src="docs/screenshots/ai.png" alt="Rubrique comparing Claude and Codex answers in the merge screen, with the Ask bar and a hover summary from a local model" width="880">

## Install

Download from [Releases](https://github.com/witseko/rubrique/releases).

| System | How |
| --- | --- |
| macOS | Open the `.dmg`, drag Rubrique to Applications |
| Debian, Ubuntu | `sudo apt install ./rubrique_X.Y.Z-1_amd64.deb` |
| Other Linux | Unpack the `.tar.gz`, run `./install.sh` |

The macOS app is signed and notarized by Apple, so it opens without a warning.

## Features

**Agents**

- **Ask** (`Cmd+K`): a request, or Shorter, Fix grammar, Table, Translate, Explain.
- **Review**: every change opens in a merge screen. Nothing applies until you accept.
- **Compare**: one request to two agents, keep the best of each.
- **Named agents and skills**: type `@ai-` to pick one. Claude Code agents work too, and skills from Claude Code, Codex (`~/.agents/skills`) and Grok. See them all in Settings → Skills.
- **Notes**: leave a note on a passage, let an agent resolve it.

Agents run only when you ask, read-only by default.

**Writing**

- Live preview: click a block to edit it. In a table, click a cell to edit
  just that cell; buttons under it add or remove rows and columns.
  HTML files get a read-only preview too.
- Unsaved changes are marked until you save: a bar beside each edited or
  new block (hover it to compare with the saved text and revert), a dashed
  line where a block was deleted, and the full list in the Changes panel.
- Side panel (the button at the right of the tabs): Structure, the file's headings (`Cmd+Shift+O`), its Notes, Tasks, or unsaved Changes.
- Voice memos (`Cmd+Shift+R`), transcribed locally with whisper.cpp.
- Drop images and audio onto the page.
- Scratch files (`Cmd+N`), each one encrypted or not, switched at any time:
  the lock button in the status bar or right-click in the explorer, and
  `Cmd+Shift+N` for a new encrypted one. Optional app PIN.
- Format JSON, SQL, XML (`Cmd+Shift+F`). Render and lint Helm charts.

**Files**

- Several folders in one explorer. Right-click a folder → New file, folder or terminal. Double-click a file, folder or tab to rename it, right-click to delete.
- Tabs keep unsaved changes, cursor and undo.
- Safe saves: your file is never reformatted; outside changes open a merge.
- Find in a file or across all folders (`Cmd+F`), match case or whole
  words, and replace one match or all of them (`Cmd+Alt+F`).
- Remote folders over SSH (off by default, Settings → Files).
- Terminals: tmux sessions as tabs, listed under the files with their agent's
  status (working, needs you, idle) and the agent sessions they ran, ready to
  resume. A `.tmux` file opens a terminal in its folder.
- Hand a step of any document to a terminal's agent: `@tmux-qa: check this`, then Send.
- Tabs grouped and colored by explorer folder; drag tabs and groups to reorder them.

<img src="docs/screenshots/themes.png" alt="One Rubrique window shown in five themes: Light, Indigo, Sepia, Midnight and High contrast" width="880">

Five themes (Indigo is the default dark one), or follow the system, or [add your own](themes/). Text size, line
numbers and other options are in Settings → Accessibility.

## Shortcuts

`Cmd` on macOS, `Ctrl` on Linux.

| Shortcut | Action |
| --- | --- |
| Cmd+K | Ask an agent |
| Cmd+F | Find |
| Cmd+E | Preview / source |
| Cmd+N | New scratch file |
| Cmd+Shift+M | Add a note |

<details>
<summary>All shortcuts</summary>

| Shortcut | Action |
| --- | --- |
| Cmd+S | Save |
| Cmd+Z / Cmd+Shift+Z | Undo / redo (in the preview too) |
| Cmd+W | Close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Cmd+, | Settings |
| Cmd++ / Cmd+- / Cmd+0 | Larger / smaller / default text size |
| Cmd+Shift+R | Record a voice memo, or stop |
| Cmd+Shift+B | Open in browser |
| Cmd+Shift+F | Format JSON, SQL or XML |
| Cmd+Shift+C | Copy the open file's path |
| Cmd+G / Cmd+Shift+G | Next / previous match |
| Cmd+Alt+F (Ctrl+H on Linux) | Find and replace |
| Alt+Up / Alt+Down | Move lines |
| Cmd+D | Duplicate lines |
| Cmd+Shift+D | Insert today's date |
| Cmd+Shift+N | New encrypted scratch file |
| Cmd+L | Lock the window (with a PIN set) |

</details>

## Set up the agents

Install the ones you want. Rubrique finds them on your `PATH` and offers the
install command when one is missing (Settings → AI). Opened from Finder or
a desktop menu, it reads the `PATH` of your login shell (`.zprofile`,
`.profile`, `.bash_profile`) and the usual install folders (including
those of nvm, fnm, Volta and a `prefix` in `~/.npmrc`, for `npm -g`), so
any other program added only in `.zshrc` or `.bashrc` is not found until
its folder is also there. The Terminals list appears once `tmux` is
installed; Settings → Terminals offers the install command.

| Program | Used for |
| --- | --- |
| `claude`, `codex`, `grok` | Ask, Compare, Notes |
| `ollama` | Local summaries, file names |
| `whisper-cli` | Transcribing memos |
| `helm` | Helm charts |
| `tmux` | Terminals |

whisper.cpp also needs its model:

```sh
curl -L --create-dirs -o ~/.local/share/whisper/ggml-small.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
```

## Build

Rust 1.95+. On Linux, also the X11, Wayland, OpenGL and ALSA dev libraries.

```sh
cargo run --release -- ~/notes
```

Before a pull request: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`, `cargo deny check`.

Releases: a `release X.Y.Z` commit on `main` (changelog section, version in
`Cargo.toml` and `Cargo.lock`) pushed together with the `rubrique-vX.Y.Z`
tag; the Release workflow builds the packages. Steps in
[`AGENTS.md`](AGENTS.md), notes in [`CHANGELOG.md`](CHANGELOG.md).

## License

[MIT](LICENSE).
