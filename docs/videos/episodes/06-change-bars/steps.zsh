# Episode 06: See what you changed before you save. Sourced by record.zsh.
start_window dark spec.md false
"$drive" activate "$pid"
sleep 0.5
"$drive" key cmd+shift+e
start_recording
glide_to 1000 600 10

# Same paragraph edit as episode 04: open the block, move to its end, type.
at edit
click_at 1192 426
sleep 0.8
click_at 700 458
"$drive" type " Log every export with the caller ID."

at edit-done
"$drive" key escape

# Hovering the checklist shows a cross at its top right.
at delete
glide_to 700 505
sleep 0.8
click_at 1261 491

# The bar beside the edited paragraph.
at hover
glide_to 310 440
sleep 0.3
glide_to 312 445 200

# Point at Revert block without clicking it, so the edit stays.
at revert
glide_to 362 675

# "Unsaved: 1 edited, 1 deleted". The microphone button sits just right of
# it at x 1042: a click there starts a voice memo.
at status
glide_to 913 699

at panel
click_at 913 699

# Cmd+W on an unsaved tab asks: Discard, Cancel, Keep for later or Save.
at close
"$drive" key cmd+w
sleep 1.5
click_at 747 400

at reopen
click_at 83 163

at recap
