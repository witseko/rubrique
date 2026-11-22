# Episode 29: OpenRig team editor. Sourced by record.zsh.
# A stand-in rig answers Validate and Launch with fixed JSON.
cp "$episode/rig" "$work/bin/rig"
cp "$episode/rig.yaml" "$notes/rig.yaml"
sample_settings="openrig_enabled=true"
start_window dark rig.yaml false
"$drive" activate "$pid"
sleep 0.5
start_recording
glide_to 790 560 10

at open
glide_to 700 420

at seat
click_at 366 594

# Runs on: claude-code is the first choice in the inspector.
at edit
click_at 944 482

at source
"$drive" key cmd+e

at validate
"$drive" key cmd+e
sleep 1
click_at 1141 78

at launch
click_at 1220 78

at confirm
click_at 412 283

at recap
