# Episode 22: Skills. Sourced by record.zsh.
# The sample profile gets the i-have-adhd skill in Claude Code's skill folder.
skill_folder="$work/profile-dark/.claude/skills/i-have-adhd"
mkdir -p "$skill_folder"
cp "$episode/skills/i-have-adhd/SKILL.md" "$skill_folder/"
install_sample_agents "$episode/answers"
start_window dark build-plan.md false claude
"$drive" activate "$pid"
sleep 0.5
start_recording
glide_to 790 560 10

at open
"$drive" key cmd+k

# The Skills button in the Ask bar.
at skills
click_at 920 156

# Picking a skill runs it at once.
at pick
click_at 937 187

at run
glide_to 700 330

at review
click_at 1163 448

# Merge screen: original left, result in the middle, Claude right.
at all
click_at 954 59

at apply
click_at 1234 59
glide_to 800 600 800

at recap
