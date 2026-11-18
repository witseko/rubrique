#!/bin/zsh
# Capture preview Find in the field guide using the current Rubrique binary.
source "$(dirname "$0")/capture.zsh"
"$drive" permissions input
start_window dark survey.md false
"$drive" activate "$pid"
sleep 0.5
"$drive" key cmd+f
sleep 0.4
"$drive" type tide
sleep 0.5
save_page editor
