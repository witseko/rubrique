# Shared macOS capture setup. Source from editor.sh, ai.sh or themes.sh,
# or from docs/videos/tools/record.zsh. Set window_width, window_height or
# text_scale before sourcing to change the sample window, and sample_source,
# sample_path or sample_tabs to use another sample folder. sample_settings
# (key=value lines) goes into the window's settings file as it is.
# Stop at the first failing command. Not `set -e`: in zsh, a failure under
# errexit inside a function exits without running the EXIT trap, which
# would leave the sample window and folder behind.
set -u
trap 'exit 1' ZERR
window_width=${window_width:-1100}
window_height=${window_height:-700}
text_scale=${text_scale:-1}
binary=$(cd "$(dirname "${1:?pass the Rubrique executable}")" && pwd)/$(basename "$1")
tools=$(cd "$(dirname "$0")" && pwd)
shots=$(dirname "$tools")
work=$(mktemp -d)
sample_source=${sample_source:-$shots/sample/notes}
sample_tabs=${sample_tabs:-ideas.md survey.md}
# This path is visible in the status bar. Take ownership before installing cleanup.
notes=${sample_path:-/tmp/field-notes}
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
cp -R "$sample_source/." "$notes"
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
    printf 'ai_summary_tool=off\nupdate_check=off\nmarkdown_only=false\nwindow_width=%s\nwindow_height=%s\ntext_scale=%s\n' \
      "$window_width" "$window_height" "$text_scale"
    for tab in ${=sample_tabs}; do
      printf 'tab=%s/%s\n' "$notes" "$tab"
    done
    printf 'ai_allowed_folder=%s\n' "$notes"
    if [[ -n "${sample_settings:-}" ]]; then
      printf '%s\n' "$sample_settings"
    fi
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

# Replace claude and codex with commands that print the answers in
# ANSWERS (claude.md, codex.md) and exit. Other agents fail, so a changed
# menu never invokes another installed agent. Each stand-in touches
# $work/NAME.done when it has answered.
install_sample_agents() {
  local answers="$1"
  for tool in gemini grok ollama; do
    ln -s /usr/bin/false "$work/bin/$tool"
  done
  for tool in claude codex; do
    cat > "$work/bin/$tool" <<'STUB'
#!/bin/sh
name=${0##*/}
case "$name" in
  claude) sleep 2 ;;
  codex) sleep 3 ;;
esac
cat "$RUBRIQUE_SAMPLE_ANSWERS/$name.md" && touch "$RUBRIQUE_SAMPLE_RUNS/$name.done"
STUB
    chmod +x "$work/bin/$tool"
  done
  export RUBRIQUE_SAMPLE_ANSWERS="$answers" RUBRIQUE_SAMPLE_RUNS="$work"
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
