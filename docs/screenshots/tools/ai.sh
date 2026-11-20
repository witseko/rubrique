#!/bin/zsh
# Capture a Compare review. The two commands print fixture text and exit.
source "$(dirname "$0")/capture.zsh"
"$drive" permissions input
# A changed menu must never invoke another installed agent during capture.
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
export RUBRIQUE_SAMPLE_ANSWERS="$tools/ai" RUBRIQUE_SAMPLE_RUNS="$work"
start_window dark todo.md false claude
"$drive" activate "$pid"
sleep 0.5
"$drive" key cmd+k
sleep 0.8
"$drive" click "$id" 640 143
"$drive" type "make it shorter"
# Open Compare with, select Codex, then run the two fixture programs.
"$drive" click "$id" 510 246
sleep 0.4
"$drive" click "$id" 505 296
sleep 0.4
"$drive" click "$id" 950 246
for attempt in {1..40}; do
  [[ -e "$work/claude.done" && -e "$work/codex.done" ]] && break
  sleep 0.25
done
if [[ ! -e "$work/claude.done" || ! -e "$work/codex.done" ]]; then
  echo "Both sample answers must finish before the Compare capture." >&2
  exit 1
fi
sleep 0.5
"$drive" click "$id" 992 485
"$drive" move "$id" 132 452
sleep 0.8
save_page ai
