# Episode 27: OpenRig seats in the explorer. Sourced by record.zsh.
# A tmux session named seat@rig is an OpenRig seat; the pod is the seat
# name up to its first '-'. These match the team in 29-team-editor/rig.yaml.
screens="$videos/agent-screens"
start_agent_session orch-lead@invoice-factory claude "$screens/lead-idle.txt"
start_agent_session dev-build@invoice-factory claude "$screens/build-working.txt"
start_agent_session dev-qa@invoice-factory codex "$screens/qa-asks.txt" "$screens/qa-after.txt"
start_agent_session review-reviewer@invoice-factory codex "$screens/reviewer-working.txt"
sample_settings="openrig_enabled=true"
start_window dark spec.md false
"$drive" activate "$pid"
sleep 0.5
# Fold the Files tree so the OpenRig section has room.
sleep 1
"$drive" activate "$pid"
sleep 0.5
click_at 22 17
sleep 0.5
start_recording
glide_to 790 560 10

# OpenRig rows with Files folded: dev-build 179, dev-qa 205, orch-lead 262.
at section
glide_to 120 140 700

at states
glide_to 160 205 900

at open
click_at 160 205

at answer
"$drive" type "1"
"$drive" key enter

at menu
right_click_at 160 179

at recap
"$drive" key escape
