# Episode 25: Terminals with agent status. Sourced by record.zsh.
# Three tmux sessions run the stand-in agent with the screens in agent-screens/.
screens="$videos/agent-screens"
start_agent_session build claude "$screens/build-working.txt"
start_agent_session qa codex "$screens/qa-asks.txt" "$screens/qa-after.txt"
start_agent_session lead claude "$screens/lead-idle.txt"
start_window dark spec.md false
"$drive" activate "$pid"
sleep 0.5
start_recording
glide_to 790 560 10

# Terminals rows: build 531, lead 560, qa 589.
at list
glide_to 120 560 700

at open
click_at 56 589

at answer
"$drive" type "1"
"$drive" key enter

at working
glide_to 120 589 600

at full
"$drive" key cmd+shift+enter

at back
"$drive" key cmd+shift+enter

at recap
