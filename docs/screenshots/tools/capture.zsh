# Shared macOS capture setup. Source from editor.sh, ai.sh or themes.sh.
set -eu
binary=$(cd "$(dirname "${1:?pass the Rubrique executable}")" && pwd)/$(basename "$1")
tools=$(cd "$(dirname "$0")" && pwd)
shots=$(dirname "$tools")
work=$(mktemp -d)
# This path is visible in the status bar. Take ownership before installing cleanup.
notes=/tmp/field-notes
if ! mkdir -m 700 "$notes"; then
  rmdir "$work"
  echo "The sample folder $notes must be free for the capture." >&2
  exit 1
fi
pid=""
stop_window() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    pid=""
  fi
}
cleanup() {
  stop_window
  rm -rf "$work" "$notes"
}
trap cleanup EXIT
cp -R "$shots/sample/notes/." "$notes"
git init --quiet --initial-branch=main "$notes"
mkdir -p "$work/bin" "$work/tmux"
swiftc -O "$tools/drive.swift" -o "$work/drive"
drive="$work/drive"

# Start one isolated window, then bind all capture commands to its process.
start_window() {
  local theme="$1" file="$2" side="$3" ask_tool="${4:-}"
  local profile="$work/profile-$theme"
  local config="$profile/Library/Application Support/rubrique"
  mkdir -p "$config"
  {
    printf 'theme=%s\nside_panel_open=%s\n' "$theme" "$side"
    if [[ -n "$ask_tool" ]]; then
      printf 'ai_tool=%s\n' "$ask_tool"
    fi
    printf 'ai_summary_tool=off\nmarkdown_only=false\nwindow_width=1100\nwindow_height=700\n'
    printf 'tab=%s/ideas.md\ntab=%s/survey.md\nai_allowed_folder=%s\n' "$notes" "$notes" "$notes"
  } > "$config/settings"
  env HOME="$profile" SHELL=/bin/false PATH="$work/bin:/usr/bin:/bin:/usr/sbin:/sbin" TMUX_TMPDIR="$work/tmux" \
    "$binary" "$notes/$file" > "$work/app.log" 2>&1 &
  pid=$!
  for attempt in {1..40}; do
    id=$("$drive" window "$pid" "$file" | cut -d' ' -f1)
    if [[ -n "$id" ]]; then
      sleep 2
      return
    fi
    sleep 0.25
  done
  cat "$work/app.log" >&2
  echo "No sample window appeared for $file." >&2
  exit 1
}

# Require a 2x display so every PNG has the same native pixel dimensions.
capture_window() {
  screencapture -x -o -l "$id" "$1"
  if [[ "$(sips -g pixelWidth "$1" | awk '/pixelWidth:/ {print $2}')" != 2200 ]]; then
    echo "Capture the 1100-point sample window on a 2x display." >&2
    exit 1
  fi
}

# Keep the 1100 x 700 content area below the native macOS title bar.
save_page() {
  capture_window "$work/window.png"
  swift "$tools/paste.swift" "$shots/$1.png" "$work/window.png" 1100 700
}
