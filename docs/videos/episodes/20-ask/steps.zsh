# Episode 20: Ask, Command K on any passage. Sourced by record.zsh.
# The stand-in Claude answers with docs/videos/answers/claude.md.
install_sample_agents "$videos/answers"
start_window dark build-plan.md false claude
"$drive" activate "$pid"
sleep 0.5
start_recording
glide_to 790 560 10

at open
"$drive" key cmd+k

# Clicking a preset runs it at once.
at presets
glide_to 499 155 900
at shorter
click_at 499 155

at answer
glide_to 900 520

at review
click_at 1163 448

# Merge screen: original left, result in the middle, Claude right.
at accept
click_at 1000 243

at reject
click_at 326 552

at apply
click_at 1234 59
glide_to 800 600 800

at recap
