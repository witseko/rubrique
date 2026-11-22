# Episode 21: Compare two agents side by side. Sourced by record.zsh.
# The stand-in agents answer with docs/videos/answers/*.md.
install_sample_agents "$videos/answers"
start_window dark build-plan.md false claude
"$drive" activate "$pid"
sleep 0.5
start_recording
glide_to 790 560 10

at open
"$drive" key cmd+k

# Cmd+K focuses the request field.
at type
"$drive" type "make it shorter"

at compare-menu
click_at 596 235
sleep 1.5
click_at 591 296

at run
click_at 1080 235

at screen
click_at 1157 448

# Merge screen: Claude left, result in the middle, Codex right.
at choose-left
click_at 328 244

at choose-right
click_at 1000 523

at result
glide_to 782 480 800

at apply
click_at 1233 60
glide_to 800 600 800
