#!/bin/zsh
# Capture the same field guide and outline in all five built-in themes.
source "$(dirname "$0")/capture.zsh"
"$drive" permissions capture
themes=(light dark sepia midnight high-contrast)
labels=("Light" "Luminous" "Sepia" "Midnight" "High contrast")
parts=()
for index in {1..5}; do
  theme="${themes[$index]}"
  start_window "$theme" survey.md true
  capture_window "$work/$theme.png"
  stop_window
  parts+=("${labels[$index]}=$work/$theme.png")
done
swift "$tools/compose.swift" "$shots/themes.png" 2200 1400 "${parts[@]}"
