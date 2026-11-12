# Changelog

All notable changes to Rubrique are listed here, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Until 1.0, a minor version (0.x.0) may change behavior or settings.

## [Unreleased]

### Added

- **Import from Sublime Text.** File > Import from Sublime Text (or the
  button under Settings > Files > Scratches) copies every unsaved Sublime
  Text tab, new tabs and files with edits not saved yet, into a new
  scratch named after the tab, so nothing is lost when you move over.
  Imported scratches are encrypted when new scratches are, and a text
  that is already a scratch is skipped, so running it again adds nothing.

### Changed

- **New file asks for a name and an extension.** New ▸ File no longer
  suggests `untitled.md`: it shows two empty fields, `name` and `ext`.
  Type the name, press Enter, then type the extension or click one of
  `md`, `txt`, `sql`, `py`, `ts`, `rs`, `go` under the fields. A name
  typed whole (`plan.md`) is created on the first Enter, and an empty
  extension makes a file without one (`Makefile`).

- **Clearer dialogs.** Every dialog (unsaved changes, delete, rename,
  open path, notes, passwords and PIN, remote folders, installs and AI
  runs) asks its question as the title and fills the usual choice with
  the accent color: Enter takes it and Esc cancels. Discard is red and set
  apart on the left of the unsaved changes dialog. In the delete dialog,
  Move to Trash is red and Enter now cancels, so a stray Enter never
  deletes a file. Running an AI tool still takes a click.

## [0.10.1] - 2026-11-11

### Fixed

- **Opening files from Finder.** Double-clicking a file, Open With, or
  dropping a file on the Dock icon opens it in Rubrique (with its folder
  in the explorer) instead of showing "Rubrique cannot open files in the
  Markdown document format". Rubrique is offered for Markdown, plain
  text, JSON and XML files in Finder and in Linux file managers, and can
  be set as the app that opens them.

## [0.10.0] - 2026-11-11

### Added

- **Highlights on and off.** A highlighter button in the status bar of
  plain text and Markdown files, `Cmd/Ctrl + Shift + H`, or View > Text
  highlights turns the text highlight rules off and on again. The button
  is filled while they are on, the rules are kept while they are off, and
  the choice is remembered for next time.
- **Touch ID for encrypted scratches** (macOS): Settings → Privacy → **Open
  encrypted scratches with Touch ID**. A locked scratch, or the Unlock
  dialog, asks for your fingerprint as soon as it shows; cancel and the
  password field is still there, with a **Use Touch ID** button to ask
  again. The scratch key is kept in this Mac's keychain and given out only
  after the fingerprints enrolled when you turned it on; add or remove one
  and Touch ID turns itself off until you set it up again. Works in the
  Rubrique app from a release, which carries the signature macOS asks for.

### Changed

- **Icons in the side panel switch.** Structure, Notes, Tasks and Changes
  are icons now, with the count after them, so the switch fits a narrow
  panel. Hover an icon to see its name.
- **A clearer Settings > Highlights.** One switch at the top turns all
  highlights on and off, and a sample you can paste your own lines into
  shows the rules coloring it as you edit them. Each rule is a short row
  with its color, pattern and how many pieces of the sample it colors;
  click a row to edit it. Add rule and the field that asks the AI for a
  rule sit at the foot of the list.
- **Preview uses the whole page.** The preview text now fills the page
  width instead of stopping at a narrow reading column, so hiding the side
  panels gives the text the room.
- **Show closed terminals.** The file filter's Hide closed terminals box is
  now Show closed terminals, ticked by default: `.tmux` files with no
  running session stay in the explorer, so you can delete the ones you no
  longer need. Untick it to hide them. A saved Hide closed terminals choice
  carries over.
- **Menus in the macOS menu bar.** On macOS the File, View and Settings
  menus are in the system menu bar at the top of the screen, after the
  Rubrique menu (About, Settings, Hide, Quit), instead of a bar inside the
  window. Other systems keep the bar in the window, which Settings can hide.
- **A clearer page for a locked scratch.** The page centers a padlock, the
  scratch's name, an outlined password field that is ready to type in, and
  a full-width **Unlock** button under it (and **Use Touch ID** under that
  when it is on). Before, the field had no visible box in most themes, so
  only a faint "Password" showed, far from the button.
- **A clearer Settings > Privacy.** A card at the top says whether the
  encrypted scratches are locked and holds the one button that matters
  now: Set password, Unlock or Lock now. Each option is one row with a
  short line on what it does and its switch or button on the right:
  Encrypt new scratches, Open with Touch ID, Encrypt existing scratches
  and the password, then the PIN, Lock after and Lock now. The paragraphs
  of text are gone.
- **No settings gear.** Settings open from the Settings menu (the macOS
  menu bar, or the bar in the window elsewhere) or with `Cmd/Ctrl+,`. On
  Linux with the menu bar turned off, the gear stays at the bottom of the
  file list so Settings can still be found.

### Fixed

- **Unsaved edits only.** With this filter on, the explorer showed only the
  top folders; the folders holding unsaved files now open on their own, so
  the files are in sight.
- **Steady find bar buttons.** The Aa, W and arrow buttons in the find bar
  no longer grow by two points when the pointer is over them.
- **Inline code in the preview.** The shaded box behind `code` now fits
  the text, without the empty strip it left underneath.

## [0.9.4] - 2026-11-11

### Added

- **Menu bar.** A File, View and Settings menu runs along the top of the
  window and repeats the common settings: theme, code colors, text size,
  text area zoom, line numbers, wrap lines, reduce motion, stronger focus,
  note marks and the file filter, along with New scratch, Open path, Save
  and Ask. A change in a menu is the same change as in Settings. Settings >
  Accessibility has a Show menu bar box to hide the bar and give the page
  its room.
- **Zoom the text area only.** `Cmd/Ctrl + Alt` with `+`, `-` and `0`, or
  `Cmd/Ctrl` and the wheel or a pinch with the pointer over the text, makes
  the editor and the preview larger or smaller (50% to 300%) while the
  explorer, tabs and bars keep their size. The same zoom is in Settings >
  Accessibility and the View menu, and is kept for next time.
- **Highlights in the preview.** The text highlight rules (IP addresses,
  ticket ids, numbers and your own) now color plain text in a Markdown
  preview too, not only in the editor.

### Changed

- **Explorer filter.** The funnel in the Files header opens a menu with
  Markdown files only, Unsaved edits only (just the files with unsaved
  changes and the folders holding them) and Hide closed terminals (`.tmux`
  files with no running session).

## [0.9.3] - 2026-11-11

### Added

- **Text highlights.** Plain text files (`.txt`, `.log` and others without
  code colors) color IP addresses, UUIDs, hashes, dates, emails and
  numbers. Settings → Highlights lists the rules: each has a name, a
  regular expression, a color from the code colors or your own, and a
  switch; add, reorder, edit or delete them. A pattern that does not work
  shows why under it and is skipped. Describe what to highlight ("ticket
  ids like PROJ-123") and the Ask tool writes the pattern as a new rule to
  check before you add it.
- **Styles in the HTML preview.** An `.html` file's preview shows the
  text colors, highlights, bold, italic, underline and strikethrough set
  in `style` attributes, `<mark>`, `<u>`, `<sup>`, `<sub>` and
  `<font color>`; centered and right-aligned titles, logos and badges
  (`align`, `text-align`, `<center>`); collapsible `<details>` sections;
  image width and height; and table cells that span columns. Colors that
  would be hard to read on your theme are toned toward its text color.
- **HTML in Markdown.** The preview of a Markdown file shows the HTML in
  it, the way GitHub does for a README: a centered logo and title, a row
  of badges, `<details>`, HTML tables, and tags such as `<b>`, `<kbd>`,
  `<sup>`, `<mark>`, `<br>` and `<img width>` inside a paragraph. Click an
  HTML block to edit its source like any other block. Scripts and style
  sheets are never run or loaded, and other tags stay as you wrote them.
- **Close a tab without saving, keep the edits.** When a tab with unsaved
  changes closes, "Keep for later" closes it without saving or dropping
  anything: the edits come back the next time you open the file, marked
  unsaved. If the file changed on disk in between, the merge screen opens
  instead of one version replacing the other. Edits of encrypted files
  are kept encrypted.
- **Wrap lines setting.** Settings > Accessibility has a Wrap lines box
  next to Line numbers. Turn it off and long lines in the editor stay on
  one line, with the editor scrolling sideways to show the rest. It is on
  by default, as before.
- **Open path.** `Cmd+O` (or a right click on the Add folder button) opens
  a file by typing or pasting its path: `~` is your home folder, a
  relative path starts at the explorer folder, and `notes.md:12` opens the
  file on line 12. A folder is shown in the explorer, added to it when it
  is not there yet. A path that does not exist is named under the field.
- **Zoom with the mouse wheel.** Hold `Cmd` (`Ctrl` on Linux) and scroll,
  or pinch on a trackpad, to make the text larger or smaller, in the same
  steps as `Cmd/Ctrl` + `+` and `-`. The page does not scroll while you
  zoom, and the size is kept for next time.

### Changed

- **Roomier find bar.** The find bar's text, field and buttons grow with
  the editor font size, and the field stands out on every theme. The
  count reads "3 of 17", Previous and Next are larger arrow buttons, and
  the Aa and W toggles show plainly whether they are on. All-files
  results are grouped under each file's name and folder, one line per
  match with its line number and the match highlighted; a click on a
  file name opens the file, a click on a line goes to the match.
- **Bigger settings window.** Settings grows with the app window, up to
  70% of its width and 80% of its height (at most 1200 by 900), so a big
  screen shows more of each group without scrolling. Small windows keep
  the old size or less.
- **Agent answers read like the preview.** In a Markdown file, the text an
  agent writes for Ask shows in the output panel with headings, bold and
  italic, lists, links and code blocks drawn, instead of raw `**` and `#`
  marks, also while it is still arriving. Copy still copies the text as
  the agent wrote it. Long answers stay smooth: they are read again only
  when new text arrives.

### Fixed

- **Review in merge keeps your block edit.** A block being edited in the
  preview is saved before the merge screen opens, so it no longer writes
  its old text over the merged result afterwards.
- **Open file highlight in the explorer.** The highlight on the open
  file's row now starts at its name, without the blank space before it.

## [0.9.2] - 2026-11-09

### Added

- **SQL colors and colored code in the editor.** `.sql` files and `sql`
  code blocks are colored: keywords in any case, strings, numbers and
  comments. In a Markdown file's source, fenced code blocks are now
  colored by their language (Python, JavaScript, TypeScript, SQL, JSON,
  XML and the others the preview colors) instead of one flat code color.

### Fixed

- **Revert block in the Changes view.** The Revert, Remove and Restore
  buttons on a change card work again; a click on them used to scroll to
  the block instead of undoing the change.
- **Linux app icon size.** On Ubuntu and other Linux desktops the icon
  fills its space like other apps (Obsidian, for one) instead of looking
  smaller: it no longer carries the margin macOS needs.

## [0.9.1] - 2026-11-09

### Added

- **VS Code code colors.** Settings → Appearance → Code colors switches
  syntax colors from the theme's to Visual Studio Code's: Dark+ on dark
  themes, Light+ on light ones. It colors the editor and preview code
  blocks; each choice shows a sample of code on its card.
- **TypeScript and React colors.** `.ts`, `.tsx` and `.jsx` files and
  `typescript`/`tsx` code blocks are colored: TypeScript's type keywords,
  built-in types and decorators, and JSX tags, attributes and the code in
  their braces.
- **Angular and HTML colors.** The source view of `.html` files colors
  tags and attributes, plus Angular templates: `{{ }}` interpolation,
  `[value]`, `(click)`, `*ngIf` and `#ref` bindings, and `@if`/`@for`
  blocks.
- **Rust colors.** `.rs` files and `rust` code blocks are colored:
  keywords, built-in types, lifetimes, `#[attributes]` and `macro!` calls.

### Fixed

- **Terminal cursor in the right place.** The cursor in a terminal tab
  now sits on the character it belongs to, instead of drifting down when
  rows above it hold symbols such as `❯` or box-drawing lines.

## [0.9.0] - 2026-11-09

### Added

- **Syntax colors for Python, JavaScript, JSON and XML.** Keywords,
  strings, numbers, comments, and the names of functions and classes are
  colored in the editor for `.py`, `.js`, `.json` and `.xml` files (and
  their relatives such as `.jsx`, `.jsonc` and `.svg`), and in preview
  code blocks marked `python`, `js`, `json` or `xml`. The colors come from
  the current theme.
- **HTML preview.** `.html` files open in the preview, drawn like
  Markdown in the theme's colors: headings, paragraphs, bold and italic,
  links, images, lists and checkboxes, quotes, code and tables. The
  Preview | Source switch (`Cmd+E`) goes back to the HTML. Styles and
  scripts are left out and nothing runs; Open in browser still shows the
  page as a browser does.
- **Terminals settings.** A new Terminals page in Settings says what
  terminals are and whether tmux was found. When it is missing it offers
  to install it (one click with Homebrew, or the `sudo apt install tmux`
  style command to copy on Linux), and Check again turns Terminals on
  without restarting Rubrique.

### Fixed

- **Codex and Claude installed with npm under nvm are found.** Opened
  from the app menu on Linux, Rubrique now also looks in the folders of
  nvm, fnm, Volta and a `prefix` set in `~/.npmrc`, which a login shell
  does not add, so `npm -g` installs show up in the agent list.
- **Terminals on Ubuntu and Debian.** The `.deb` package now recommends
  `tmux`, so `apt` installs it with Rubrique and the Terminals list shows.

## [0.8.9] - 2026-11-08

### Added

- **Undo and redo in the preview.** Cmd+Z brings back a block you
  deleted (or undoes a block edit, a table row or column, or Prettify),
  and Shift+Cmd+Z or Cmd+Y does it again. It is the same history as the
  source editor, so you can switch views and keep stepping through it.
- **See what you changed before you save.** In the preview, a bar beside
  a block marks it as edited or new since the last save, and a dashed
  line shows where a block was deleted. Hover the bar to compare the saved
  and current Markdown and put the block back with one click. The status
  bar counts the changes ("Unsaved: 2 edited, 1 deleted"); click it for
  the new Changes panel, which lists every block that will change on Save,
  with Revert, Discard all and Save. Undoing back to the saved text takes
  a change off the list, and saving clears it.

### Fixed

- **Tables with long paths or links fill the page again.** A cell with
  an unbreakable run such as `/api/v2/some-long-path` no longer pushes its
  text to the left and widens the column, which left empty room on the
  right of the table.

## [0.8.8] - 2026-11-08

### Added

- **Delete a block from the preview.** Hover a block and click the cross
  beside its top right corner, or press Delete next to Done while editing
  it. The blank lines around it go too; save to keep the change.
- **Insert a block between blocks in the preview.** Hover the gap
  between two blocks (or above the first one) and an accent line with a
  plus shows; click it to type a new block right there.

### Fixed

- **Symbols like ✔ no longer show as empty boxes.** Characters the
  built-in fonts lack, such as check marks, stars, emoji and Chinese or
  Japanese text, are now drawn with a font from your system.

## [0.8.7] - 2026-11-08

### Added

- **Drag folders and terminals into your own order.** Drag a root folder
  in Files, or a session in Terminals, up or down; a line shows where it
  will land. Tab groups follow the new folder order, and the terminal
  order is remembered between runs (new terminals still show on top).

### Fixed

- **Enter no longer deletes a cleared block.** Clearing a block in the
  preview and leaving it keeps the block as it was, even after pressing
  Enter in the empty box; before, the block was replaced by a blank line.
  Blank lines typed at the start or end of a block are dropped when you
  finish, so they no longer leave extra space in the file.

## [0.8.6] - 2026-11-08

### Changed

- **The Markdown filter is a button in the Files header.** A funnel next
  to Refresh turns "Markdown only" on and off and stays highlighted while
  it is on, instead of a checkbox above the tree, so the tree starts right
  under the header.
- **The first run opens no folder.** Rubrique used to show the folder it
  was started from (often your home folder, or `/` from the Finder); now
  the explorer starts empty with an "Add folder..." button. Folders you
  add are reopened next time as before.

### Fixed

- **Arrows no longer show as empty boxes.** The link marker after symlinked
  files and folders in the explorer (`↗`, for example on `/var` on macOS)
  and the `↑ up` button when browsing a remote host now draw properly.

## [0.8.5] - 2026-11-08

### Changed

- **App icon**: the capital "R" with the marker tip now sits on the Indigo
  theme's colors, a deep indigo tile with a lavender letter and a yellow
  tip, like the earlier lowercase icon, instead of the paper and brown of
  the Light theme.
- **Conflicts on the merge screen are tinted a pale accent** (a light
  indigo in the Indigo theme) instead of the warning amber, which turned
  olive on dark themes. Plain changes keep the accent tint.
- **Larger, roomier text.** Labels, buttons and the explorer are a point
  bigger (14 instead of 13), hints are 11 instead of 9, the explorer's
  section titles 12 instead of 11, and the editor font
  starts at 14 points for new installs. Lines in the source editor and the
  block editor are one and a half times the font size apart instead of
  touching, with the line numbers centered on them, and the preview reads at
  16 points with headings to match. The text size setting still scales all
  of it.

### Fixed

- **The Terminals block no longer jumps when a terminal is opened.** The
  status bar kept the height of its buttons only while a text file was
  open; a terminal tab (or a status message) made it shorter, which moved
  the explorer's bottom edge and the Terminals block docked there. The
  bar now keeps one height.
- **Editing a block in the preview keeps its place.** The box that opens
  on a click uses the page's text size and line height instead of the
  editor font, so the text no longer shrinks and the block keeps about its
  height; it no longer puts a title over an existing block, so the first
  line stays where it was; and the cursor lands on the line that was
  clicked instead of at the end of the block, which scrolled a long list
  or code block away.

## [0.8.4] - 2026-11-08

### Changed

- **Folder colors and unsaved files no longer look alike in the explorer.**
  A folder's color is a bar on the left edge of its row (the counterpart of
  the colored line on its tabs) instead of a dot after its name, and an
  unsaved file keeps its bullet after the name but in the text color, like
  the dot on its tab, instead of turning green (the same green as the
  second folder's color).

## [0.8.3] - 2026-11-08

### Changed

- **App icon and name**: the capital "R" icon with the marker tip is back,
  and the app calls itself Rubrique everywhere you see its name: the window
  title, Settings, the macOS app (now `Rubrique.app`) and disk image, the
  Linux launcher and the release notes. The `rubrique` command, settings
  folder and file formats are unchanged.

## [0.8.2] - 2026-11-08

### Added

- **Version in Settings**: the version of rubrique you are running is shown
  at the bottom of the group list in Settings.

### Changed

- **macOS disk image**: the `.dmg` opens as a 600 x 400 window with the
  app and the Applications folder as large icons and an arrow between
  them, instead of a Finder window of whatever size was last used with
  the two icons in a corner.

## [0.8.1] - 2026-11-08

### Fixed

- **Agents found when started from Finder or a launcher**: rubrique now
  asks your login shell for its `PATH` at startup, so `claude`, `codex`,
  `ollama` and the others installed in `~/.local/bin`, Homebrew or npm's
  folder are found when the app is opened from Finder, the Dock or a
  desktop menu, not only from a terminal. Folders set in `.zprofile`,
  `.profile` or `.bash_profile` count; `.zshrc` and `.bashrc` do not.

## [0.8.0] - 2026-11-07

### Added

- **Indigo** replaces the Dark theme (same id, so saved choices keep working) and is the default for new installs. **System** shows Light or Indigo. New lowercase "r" app icon in indigo.
- **Rename and delete folders**: right-click a folder in the explorer, or double-click it to rename in place, like files.
- **Move tabs and groups**: drag a tab sideways to reorder it (inside its group), or drag a group chip onto another group to reorder it (and its folder in the explorer).
- **Dragging a tab or group shows where it lands**: a half visible copy follows the pointer, the original is dimmed in place, and a line in the strip marks the spot where it will be inserted. Dragging a tab to the left over other tabs no longer drops the drag halfway.
- **Exit terminal** in the right-click menu of a terminal tab and of a `.tmux` file.
- **Encrypt or decrypt a scratch from the explorer**: right-click a file in
  the scratch folder for **Encrypt** or **Remove encryption**, open or not.
  Both ask for the scratch password when it was not entered yet (before,
  removing encryption only worked after unlocking in Settings), so a
  scratch can change at any time, whatever **Encrypt every new scratch** says.
  An encrypted scratch shows a padlock after its name, in its tab and in
  the explorer.
- **Add remote folder shows what is there**: pick a computer and the dialog
  lists the folders of your home there. Click one to look inside it, "up"
  to go back, or type a path; the list follows what you type, Tab completes
  the one name left, and the Add button says what it adds. A computer that
  already has a folder in the explorer is picked and listed when the dialog
  opens.
- **Settings → Files checks the ssh program**: under the field, "Found:
  /usr/bin/ssh", or why the setting cannot run (a command line such as
  `ssh myhost` is not a program).
- **Edit table cells in the preview**: click a cell to change just that
  cell, in a box that wraps like the cell does. Enter, Esc or a click
  outside finishes; a click on another cell moves there. Buttons under the
  table add a row below, remove the row, add a column after, or remove the
  column of that cell, and a new row or column opens its cell for typing
  right away. Pipes you type are escaped and line breaks become spaces, so
  the table stays whole. Tables wider than the page scroll sideways, and a
  click beside the cells still opens the whole table's Markdown.

### Changed

- **Signed macOS app**: the app is signed with a Developer ID and notarized by Apple, so it opens without the Gatekeeper warning and the `xattr` step is no longer needed.
- **Status bar buttons are icons**: Record (a microphone), Add audio file
  (a waveform with a plus), Open in browser (a window with an arrow) and
  Format (text lines) are icon buttons, grouped by a thin line, with
  **Ask** as the one labelled button in the corner. Rest the pointer on a
  button for its name and shortcut. While recording, the microphone button
  shows the elapsed time with a red dot.
- **Preview | Source is two icons** (the eye and the code brackets), which
  leaves about 110 px more for tabs.
- **Group chips** in the tab strip have the same corners as the buttons
  around them instead of a pill, and start with the folder's color dot
  from the explorer.

### Fixed

- **"No folder ~ on host" when the ssh program setting was wrong**: an ssh
  program that cannot start was reported as a missing folder (and could make
  an open remote file look deleted). The dialog now says the program cannot
  run, with a link to the setting, and checks the setting before connecting.
- **Hosts with a `RemoteCommand`** in `~/.ssh/config` (one that opens tmux,
  say) failed with "Cannot execute command-line and remote command". rubrique
  turns the remote command and the terminal request off for its own calls.

## [0.7.1] - 2026-11-06

### Added

- **Skills from more agents**: besides rubrique's own folder and Claude
  Code's, skills now come from `~/.agents/skills` (the shared folder Codex
  reads) and Grok's `~/.grok/skills`. Each source can be switched off.
- **All skills in Settings → Skills**: every loaded skill in one list, with a
  filter field and a button per source. Click a skill to see where it lives,
  the start of its instructions, and to open its SKILL.md or folder. A skill
  linked into several folders shows once, with a badge per source; a
  different skill with a taken name is listed as skipped.
- **Encrypt one scratch at a time**: a scratch can be encrypted or plain on
  its own, without turning on encryption for all of them. On an open
  scratch, the **Encrypt** button (a padlock) in the status bar seals it
  with the scratch password; on an encrypted one it reads **Encrypted**,
  and clicking it offers **Remove encryption**. `Cmd/Ctrl+Shift+N` makes a
  new encrypted scratch, and right-clicking the New scratch button offers
  **New plain scratch** and **New encrypted scratch**. With no password yet,
  rubrique asks you to choose one first; when locked, it asks for it. Unsaved
  edits stay in the tab, and their kept draft is sealed or made plain too.
  Settings → Privacy → **Encrypt every new scratch** still sets what
  `Cmd/Ctrl+N` makes.
- **Find options**: **Aa** (match case) and **W** (whole word) toggles in
  the find bar, for the open file and for All files.
- **Replace**: `Cmd+Alt+F` (`Ctrl+H` on Linux), or the **Replace** switch
  in the find bar, adds a row under the search field. **Replace** (Enter)
  changes the current match and moves to the next one; **Replace all**
  (`Cmd+Enter`) changes every match. Each is one undo step, so one `Cmd+Z`
  brings the old text back. The status bar says how many were replaced.
- Opening the find bar with text selected (one line) searches for it.

### Changed

- **Settings → AI** is simpler. The two choices come first: **Ask and
  notes** (the tool behind `Cmd+K`, Resolve notes and Compare) and
  **Summaries and names**, each a drop-down over the tools that were found.
  Under them, **Tools** lists Claude Code, Codex, Grok and the local model
  with a check or a warning dot, where each was found, Ask and Summaries
  tags, and the install or model download prompt when something is missing.
  The command templates are folded behind a **Command** button on each row;
  a command without `{prompt}` opens by itself with the warning. Safety is
  one line with the folder count and **Forget allowed folders**.
- **Flag chips** above a command: **Model** for Claude Code, Codex and the
  local model, **Permissions** for Claude Code and **Sandbox** for Codex.
  A chip rewrites just its flag in the command; everything else you typed
  stays, and the command field remains the source of truth.

### Fixed

- `Cmd+Shift+F` (Format) also opened the find bar.

## [0.7.0] - 2026-11-06

### Added

- **Rename from a tab**: double-click a tab (or right-click → **Rename**) to
  edit the file's name right in the tab. Enter or clicking away renames,
  Esc cancels.
- **New ▸ File / Folder / Terminal** in a folder's right-click menu: a name
  field opens at the top of the folder with a free name filled in (the part
  before the extension selected). Enter creates it, Esc cancels. The + in
  Terminals names a new session the same way, at the top of the list.
- **Terminals list**: right-click a terminal → **Show in files** opens the
  tree down to its `.tmux` file (or the folder it runs in) and highlights it.
  Double-click a terminal to rename it in place; one opened from a `.tmux`
  file renames that file instead, so the two stay linked.
- **Hand steps to terminals with `@tmux-`**: in any document, write
  `@tmux-<terminal>: what to do` in a step (typing `@tmux-` suggests your
  `.tmux` files and running terminals). The side panel's new **Tasks** view
  lists them; **Send** pastes the instruction, the step it sits in and the
  document's path into that terminal's agent, then marks the mention sent
  with a hidden comment. It is pasted only while an agent (Claude Code,
  Codex, ...) waits at its prompt there, never into a bare shell; until then
  it waits, and the terminal opens so you can start one.
- **Tab groups**: with more than one folder in the explorer, each folder
  gets a color (a dot after its name) and its tabs sit together behind a
  chip in that color, each with a thin line in it. A new tab opens at the end
  of its group. Click a chip to fold the group's tabs.
- **Agent sessions per terminal**: rubrique notices which Claude Code or
  Codex conversation runs in each terminal and remembers it. A terminal
  that ran some gets a fold arrow in the list; unfold it to see them
  (title and when). Click one to resume it in that terminal (or in a new one
  when the terminal is busy); right-click for **Resume in new terminal** and
  **Copy session id**.

### Changed

- **Side panel**: Structure and Notes share one panel on the right, opened
  from the button at the right end of the tab bar. A switch at its top picks
  Structure or Notes, a cross closes it, and it stays as you left it. Notes
  no longer open on their own: a dot on the button says the file has notes,
  and adding a note opens them. The Structure button left the status bar;
  `Cmd+Shift+O` still toggles Structure.
- **Explorer sections**: Files and Terminals each have a header that folds
  them. The Files header holds Add folder, New scratch and Refresh. Terminals
  moved under the files, docked at the bottom of the panel: drag its top edge
  to resize it. While folded, its header still shows how many agents need
  you or are working.
- Encryption libraries updated (argon2 0.6, chacha20poly1305 0.11).
  Encrypted scratches and the app PIN made with earlier versions keep
  working; a test checks values made with the old libraries.
- Terminal status is a colored dot before the status text, like herdr:
  blue working, orange needs you, green idle, gray shell; a hollow ring for
  a `.tmux` file that is not running. The terminal icon is one quiet color.

### Fixed

- The Terminals list was empty with tmux 3.8, which prints tabs in its
  output as `_`; the session list now uses `:` between fields.

## [0.6.5] - 2026-11-06

### Added

- **Terminals**: tmux sessions open as tabs, and you type straight into
  them (vim and agents work: tmux does the terminal). The explorer lists
  every session at the top under **Terminals**, newest first, with when it
  was opened and what runs in it. For coding agents (Claude Code, Codex,
  Grok and others) it shows **working**, **needs you** (a question or
  permission prompt on screen) or **idle**, and the terminal icon takes the
  matching color. **New terminal** starts one; right-click a folder →
  **New terminal here** adds a `.tmux` file that opens a terminal in that
  folder. Closing a tab leaves the session running. Local folders only;
  needs `tmux`.
- **Structure** (`Cmd+Shift+O`, or **Structure** in the status bar): the
  headings of the open Markdown file, indented by level, in the right panel
  where the notes are. Click a heading to jump to it in the preview or the
  editor; the section on screen is highlighted.

## [0.6.0] - 2026-11-05

### Added

- **Delete files** from the explorer: right-click a file → **Delete…**. Local
  files go to the Trash; files in remote folders are deleted for good, and the
  dialog says so first. Its tab closes.
- **Rename in place**: double-click a file in the explorer to edit its name
  right in the tree. Enter or clicking away renames, Esc cancels. **Rename…**
  in the right-click menu still opens the dialog with **Suggest**.
- **Agent suggestions**: typing `@ai-` in the editor, the preview's block
  editor or the note dialog lists matching agents under the cursor. Up and
  Down choose, Enter or Tab inserts, Esc hides. In the Ask bar, suggestions
  now also work after other words, not only at the start.

### Fixed

- Tables in the preview no longer squeeze every column to a word or two.
  Short columns keep their width and long ones wrap to fit the page.

### Changed

- A shorter README.

## [0.5.0] - 2026-11-03

### Fixed

- Scrolling a page to its end with a trackpad no longer overshoots and
  springs back (the "jump" at the bottom of the preview).
- Hovering a tab shifted the tabs to its right by a few pixels while the
  close cross was shown; the tabs after the active one sat too close too.

### Added

- **Helm charts**: `.yaml`, `.yml` and `.tpl` files (and a chart's
  `templates/NOTES.txt`) get syntax colors: keys, `#` comments, list dashes,
  anchors, block scalars, and Helm template actions (`{{ }}`, `{{- -}}`,
  `{{/* */}}`), also when they span lines. In a chart folder (one with
  `Chart.yaml`), the **Helm** button in the status bar runs `helm template`
  and shows the rendered files, colored, in a panel below the page; **Lint**
  runs `helm lint`. With a `values-*.yaml` file of the chart open, it is
  passed with `--values`. helm reads the saved files, so the panel says when
  files in the chart have unsaved changes. Not available for remote folders.
- **Encrypted scratches**: Settings → **Privacy** → **Encrypt new scratches**
  asks for a password, and every new scratch is saved encrypted with it
  (XChaCha20-Poly1305, key from Argon2id). They open after you enter the
  password, once per session; a tab waiting for it shows a password field.
  Unsaved edits of an encrypted scratch are kept as an encrypted draft. AI
  never gets their text: Ask, Compare, Notes, summaries and name suggestions
  are off for them, and search across files matches only their names. Also
  **Lock scratches**, **Change password…** and **Encrypt existing scratches**.
  The settings file keeps only a salt and a check value, never the password.
- **App PIN**: Settings → **Privacy** → **Set PIN…**. rubrique asks for it at
  start and after it sits idle (**Lock after** 1, 5, 15 or 30 minutes, or
  off); `Cmd/Ctrl+L` locks now. After five wrong PINs it waits 30 seconds.
  The PIN locks the window only, it does not encrypt files. Locking also
  locks encrypted scratches again.
- **Remote folders over SSH** (optional, off by default): Settings → Files →
  **Open folders on other computers over SSH**, then **Add remote folder**
  in the explorer. rubrique runs the system `ssh` (your `~/.ssh/config`,
  keys, ssh-agent and jump hosts work; password logins do not) and shares
  one connection per host. Remote files open, save safely (temp file and
  rename on the server, permissions and symlinks kept, a cut-off upload
  changes nothing), and are checked for outside changes every 2 seconds,
  with the merge screen when both sides changed. Images linked from a remote
  note are downloaded for the preview, **Find → All files** searches remote
  folders on the server, and agents run on a local copy of the note's folder.
  Remote folders and tabs come back at the next start (saved as
  `ssh://host/path`). Adding images, audio or voice memos to remote notes is
  not supported yet.
- **Unsaved files are green in the explorer**, with a dot after the name,
  for every tab with unsaved changes, not only the open one.
- **Double-click a file in the explorer to rename it.** A single click
  still opens it.
- **Files with notes get a speech bubble** before their name in the
  explorer. Turn it on in Settings → Files → **Mark files that have notes**.
  Files are checked when shown and read again only after they change.
- **Click the empty space under the last block** in the preview to type a
  new block there. It goes to the end of the file after a blank line; an
  empty box adds nothing.
- **Tables** (`| a | b |` with a `| --- | --- |` row) show as a grid in the
  preview, in AI answers and in **Open in browser**, with bold headers,
  striped rows and the column alignment from the colons. A table can follow
  a paragraph line directly, as on GitHub.
- **Find** (`Cmd/Ctrl+F`): a bar at the top right of the page. Matches in
  the open file are tinted, the current one in the accent color; Enter,
  Shift+Enter, `Cmd/Ctrl+G` and `Cmd/Ctrl+Shift+G` step through them, and
  Esc closes the bar with the cursor on the match. **All files** searches
  every text file under the explorer's folders (hidden entries and files
  over 1 MB skipped) by name and by content, lists the first 300 hits with
  their line, and opens the file at the match on a click. Find works on the
  source, so a preview switches to it.
- **Line numbers** in the source editor: Settings → Accessibility → **Line
  numbers**. A wrapped line is numbered once.

- **Agents**: named helpers with their own personality, skills, AI tool and
  model. Each one is a folder with an `AGENT.md` file in the agents folder.
  Claude Code's agents in `~/.claude/agents` work too.
- In Ask (`Cmd/Ctrl+K`), type `@` to pick an agent, or use **Agents…**.
  Mentions start with `ai-` (`@ai-editor`), so they never clash with people's
  names. The agent's personality and skills go with the request, and its
  model is passed to the tool (`--model`, or the model in `ollama run`).
  **What Claude gets** shows the agent and its skills, and the command
  with the model.
- **Settings → Agents**: the agents folder, **Also use Claude's agents**, the
  list of agents with **Open**, and **Create example agent**.

### Changed

- rubrique is now described as **a fast editor, agentic on demand**.
  Markdown is what it does best, not what it is. Which agents, and on which
  systems, are details that may change. The README, the package descriptions
  and the repository description say so; [`docs/positioning.md`](docs/positioning.md)
  keeps the wording in one place. Nothing in the app changed.

## [0.4.4] - 2026-10-31

### Changed

- The **Ask bar shows what the tool gets** before it runs: open **What
  Claude gets** for the request, the text, rubrique's instructions, the
  skill, the command and the folder. The first run in a folder asks there
  instead of in a separate dialog.

### Fixed

- Selecting text across several list items, task items or quote lines in
  the preview and then adding a note or asking AI said "Cannot find the
  selection in the file".
- The preview's **+ Add note** button showed behind the open Ask bar.

## [0.4.3] - 2026-10-31

### Changed

- Every Markdown file opens in **preview**. A **Preview | Source** switch at
  the right end of the tab bar (or `Cmd/Ctrl+E`) shows the source, for that
  tab only. New scratch files open on their source, ready to type. The old
  status bar Edit / Preview button is gone.
- **Quitting keeps unsaved edits** as drafts and restores them on the next
  start, without asking. The files themselves are not touched until you
  save. If a file changed on disk meanwhile, the merge screen opens.

## [0.4.2] - 2026-10-30

rubrique is now described as what it has become: an AI-assisted Markdown
editor. This version adds voice memos, transcribed on your computer.

### Added

- **Record** (`Cmd/Ctrl+Shift+R` or the status bar button): records into a
  WAV file in a `memos` folder next to the note and links it at the cursor.
  The recording bar shows the time, a level meter, **Stop** and **Cancel**.
- **Drop audio** (WAV, M4A, MP3, OGG, Opus, FLAC, WebM, AAC) on the page to
  add it as a memo. **Audio…** in the status bar does the same. Drops do not
  work on Wayland yet.
- **Play** memos in the preview, with play, pause and a seek bar. WAV plays
  inside rubrique; other formats open in your audio player.
- **Transcribe** a memo: the text goes under it, through the merge screen.
  The default tool is whisper.cpp (`whisper-cli`); any tool that prints text
  works.
- **Settings → Voice memos**: memos folder, copy or link dropped files, the
  transcription command, and **Transcribe after recording**.
- **Drop images** on the page: they are copied into an `images` folder next
  to the note and linked where the cursor is.
- **Install** a missing tool with one click: when whisper.cpp, its model,
  Claude Code, Codex or Ollama is not found (on Transcribe, Ask, or in
  Settings), rubrique offers its install command (`brew`, `npm`, `curl` or
  `ollama pull`) and shows the command first. Install scripts piped to a
  shell, and tools without a known command, get **Copy install command**
  and **Open install page** instead.
- **Settings → Accessibility**: text size for the whole window (also
  `Cmd/Ctrl` + `+`, `-`, `0`), editor font size, **Reduce motion** and a
  **Stronger focus outline**.

### Changed

- **Settings** is now a window over the app, with groups on the left:
  Appearance, Files, AI, Skills and Voice memos. A click outside or Esc
  closes it.
- macOS: release builds are signed with a Developer ID and notarized by
  Apple once the signing secrets are set. Until then the release notes say
  how to open the app the first time.
- AI answers are shown formatted: headings, lists, code blocks and links,
  with **Copy** for the raw text. Code blocks in an answer have their own
  **Copy**. Hover summaries show bold, italic and code.
- macOS asks for the microphone the first time you record.
- New dependency `cpal` for the microphone and speakers. Building on Linux
  needs `libasound2-dev`.

### Fixed

- Clicking the close cross of a tab with unsaved changes did nothing. It now
  asks to save, discard or cancel.
- Clicking **+ Add note** after selecting text in the preview opened the
  paragraph for editing instead of the note dialog.
- A block right after a code block inside a list or quote was drawn on top
  of the code block's header.

## [0.4.1] - 2026-10-28

### Added

- **Tabs**: every file opened from the explorer gets its own tab, right
  after the active one; opening a file that has a tab switches to it. Each
  tab keeps its unsaved changes, cursor, undo history and scroll position,
  so switching never asks to save. A dot marks unsaved changes. Close with
  the cross, a middle click or `Cmd/Ctrl+W` (unsaved changes ask first);
  right-click for **Close others**, **Close to the right** and **Copy
  path**; `Ctrl+Tab` / `Ctrl+Shift+Tab` move between tabs. Tabs reopen at
  the next start.
- **Images**: PNG, JPEG, GIF, WebP, BMP, ICO and TIFF files open in an image
  view, fitted to the page or at actual size (click the image or use the
  status bar button), with the size in pixels and the file size in the
  status bar. Animated GIF and WebP play.
- **Videos**: MP4, MOV, WebM, MKV, AVI and other video files show a card
  with **Play in video player** (the system's default player) and **Play
  in browser**.
- Markdown images `![alt](path)` show in the preview (relative paths start
  at the file's folder); a video path shows a card with **Play**. Open in
  browser shows them as `<img>` and `<video>`. Web images are not loaded in
  the preview.

### Changed

- New explorer icons in the style of Apple's symbols: **Add folder** is a
  folder with a solid plus badge, **New scratch** is a square with a pencil
  whose tip uses the accent color.
- Closing the window with unsaved changes in several tabs asks about each file in turn.
- New dependencies `egui_extras` and `image`, only to decode images.

### Fixed

- Undo after opening another file could bring back text from the file
  shown before; each file now has its own undo history.

## [0.4.0] - 2026-10-28

AI-first, still small: rubrique runs the AI command-line tools you already
have (Claude Code, Codex, Grok, or a local model through Ollama or
llama.cpp). It holds no API keys, and every change is shown in the merge
screen before it touches your text.

### Added

- **Ask** (`Cmd/Ctrl+K`, or the status bar button): ask an AI tool to change
  the selection (or the whole file). Quick actions: Shorter, Fix grammar,
  Table, Translate, Explain. The output streams into a panel with **Stop**;
  **Review in merge** shows each change next to the original, and nothing
  changes until you press **Apply**. The first run in a folder shows the
  exact command and asks first.
- Compare tools: pick **Compare with** in the Ask bar to run a second tool
  at the same time. The output panel shows both answers side by side, and
  **Compare in merge** puts them left and right of the result, with the
  time each took; every block where they differ is a choice.
- Resolve notes with AI: **Resolve** on a note card, or **Resolve all** in
  the Notes panel, sends the file and the note to the default tool. While it
  runs the card shows the time and **Stop**; the answer opens in the merge
  screen, where the resolved notes are removed once you apply.
- Skills: reusable instructions for Ask, one folder per skill with a
  `SKILL.md` file, the same format as Claude Code skills. Pick one from
  **Skills…** in the Ask bar. Settings → AI → Skills sets the folder
  (default `skills` in the config folder), can include Claude's own
  `~/.claude/skills`, and can create an example skill.
- Hover summaries: rest the pointer on a file in the explorer to see a
  two-sentence summary, made by a local model (Ollama or llama.cpp) by
  default and cached by content, so a file is summarized again only after
  it changes. Pick the tool or turn it off in Settings → AI → Summaries use.
- Smart scratch names: after you save a scratch that still has its random
  name, the summaries tool suggests a name from its content, like
  `2026-10-28-beta-launch-checklist.md`, in a bar with **Rename** and
  **Not now**. The Rename dialog has **Suggest** too.
- Settings → AI: pick the default tool (Claude, Codex, Grok or Local; ones
  not installed are greyed out), edit each tool's command (Installed with
  its path on hover, or Not found with a hint), pick the tool for summaries
  or turn them off, and forget the folders where AI runs were allowed.
  Built-in commands run read-only: Claude in plan mode with rubrique's
  instructions as a system prompt, Codex in a read-only sandbox.
- Code blocks in the preview: hover one to get **Copy**, and **Prettify**
  for ` ```json `, ` ```sql ` (also `psql`, `postgres`) and ` ```xml `
  blocks, which formats the code in the file like Format does. The
  language label and buttons sit in a slim row above the code.

### Changed

- Add folder and new scratch icons redrawn: a plain folder and a page with a
  separate "+" badge, and thinner lines on all icons so they stay sharp.

### Fixed

- Long lines in the merge screen's columns were stretched across the
  column with wide gaps between letters.

## [0.3.0] - 2026-10-28

### Added

- App icon: a lowercase "m" ending in a marker tip. Shown on the window,
  in the macOS app and in Linux menus.
- Scratch files: the page-plus icon or `Cmd/Ctrl+N` creates an empty note with a
  random name like `2026-10-28-brave-otter.md` in a scratches folder
  (set in Settings → Files).
- **Rename…** in a file's right-click menu.
- The explorer's top row uses icons: add folder, new scratch and refresh
  (hover for a tip), like the settings gear.
- **Open in browser** (`Cmd/Ctrl+Shift+B`, or the status bar button): shows
  the Markdown as a full web page in the current theme's colors, or an HTML
  file as it is, in the default browser. Unsaved changes are included.
- Live check for changes by other programs (every 2 seconds). A file without
  unsaved edits reloads with a status message. With unsaved edits a bar
  offers **Compare and merge…**, **Reload theirs** or **Keep mine**, and you
  can keep typing meanwhile.
- Merge screen: the file on disk, the result and your version side by side
  (**Swap sides** flips them). One-sided changes merge by themselves; for
  conflicts pick left, right or both, or edit the result, then **Apply**.
- **Format** (`Cmd/Ctrl+Shift+F`, or the status bar button in `.json`,
  `.sql` and `.xml` files): pretty-prints JSON, XML, and SQL in
  PostgreSQL / pgFormatter style. Works on the selection too, guessing its
  language, so code blocks in notes can be formatted. Undo with `Cmd/Ctrl+Z`.
- Edit in the preview: click a block (paragraph, heading, list, quote, code)
  to edit just that block's Markdown in place; Esc or a click outside shows it
  rendered again.
- Notes: drag to select text in the preview, then **+ Add note** (or
  `Cmd/Ctrl+Shift+M`, also in Edit mode). The note is stored in the file as a
  hidden comment after the text, highlighted in the preview with the note on
  hover, and listed in a **Notes** panel with Edit and Delete.
- The preview hides HTML comments; the editor dims them.
- Dependency security checks: cargo-deny in CI (known security problems,
  licenses, crate sources), weekly and on every dependency change, and
  Dependabot updates for crates and GitHub Actions.

### Changed

- The settings gear is redrawn to match the app icon, like the new explorer
  icons: heavy rounded lines with one detail in the theme's accent color.

### Fixed

- The unsaved-changes dot showed as an empty box.
- A long file path in the status bar ran into the word count; it is now
  shortened with "…".
- Long names in the explorer wrapped onto two lines with the folder arrow
  drawn over them; rows now stay on one line and the explorer scrolls sideways.

## [0.2.0] - 2026-10-28

### Added

- Themes: Light, Dark, Sepia, Midnight and High contrast are built in, and
  **System** follows the operating system. Add your own as small `.theme`
  files in the config folder.
- Settings panel (gear at the bottom of the explorer, or `Cmd/Ctrl+,`) with
  theme cards and **Reload themes**.
- Markdown syntax colors in the editor.
- Several folders in the explorer at once: **+ Add folder…** opens the system
  folder window; right-click a folder to remove it. Several folders can also
  be passed on the command line, and the list is remembered.
- Click **Edit / Preview** in the status bar to switch modes.

### Changed

- Cleaner preview: larger text with more line spacing, lines under the top
  headings, drawn bullets and checkboxes, a colored quote bar, outlined code
  blocks.
- Borderless editor and preview page with margins.
- The Folder bar at the top is gone; use **+ Add folder…** instead.
- Settings store explorer folders as `folder=` lines. An older
  `last_folder=` line is still read.

### Dependencies

- Added `rfd` for the native folder window.

## 0.1.0 - 2026-10-27

### Added

- First version: file explorer with a Markdown-only filter and copy-path menu,
  plain-text editor with a rendered Markdown preview, atomic saves, unsaved
  changes prompt, reload on outside changes, and line shortcuts.

[Unreleased]: https://github.com/witseko/rubrique/compare/rubrique-v0.5.0...HEAD
[0.5.0]: https://github.com/witseko/rubrique/compare/rubrique-v0.4.4...rubrique-v0.5.0
[0.4.4]: https://github.com/witseko/rubrique/compare/rubrique-v0.4.3...rubrique-v0.4.4
[0.4.3]: https://github.com/witseko/rubrique/compare/rubrique-v0.4.2...rubrique-v0.4.3
[0.4.2]: https://github.com/witseko/rubrique/compare/rubrique-v0.4.1...rubrique-v0.4.2
[0.4.1]: https://github.com/witseko/rubrique/compare/rubrique-v0.4.0...rubrique-v0.4.1
[0.4.0]: https://github.com/witseko/rubrique/compare/rubrique-v0.3.0...rubrique-v0.4.0
[0.3.0]: https://github.com/witseko/rubrique/compare/rubrique-v0.2.0...rubrique-v0.3.0
[0.2.0]: https://github.com/witseko/rubrique/releases/tag/rubrique-v0.2.0
