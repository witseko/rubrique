# Episode 17: Git from the status bar. Sourced by record.zsh.
# The sample folder gets a commit identity and a first commit, so the
# branch menu can create branches and commit.
git -C "$notes" config user.name "Invoice Team"
git -C "$notes" config user.email "team@example.com"
git -C "$notes" add --all
git -C "$notes" commit --quiet -m "add the invoice export spec"
start_window dark spec.md false
"$drive" activate "$pid"
sleep 0.5
start_recording
glide_to 790 560 10

# "Not trusted" is the branch button at the left of the status bar.
at trust
click_at 72 699

at trust-confirm
click_at 299 670

# The same button now shows the branch and opens the branch menu.
at branch
click_at 72 699
sleep 0.8
click_at 117 622
"$drive" type "export-logging"

at branch-create
"$drive" key enter

# Same paragraph edit as episode 04, then save.
at edit
"$drive" key cmd+shift+e
sleep 0.5
click_at 1192 426
sleep 0.8
click_at 700 458
"$drive" type " Log every export with the caller ID."
"$drive" key escape

at save
"$drive" key cmd+s

at commit
click_at 72 699

# Commit... in the branch menu, then the commit message field.
at message
click_at 94 562
sleep 0.8
"$drive" type "log every export with the caller ID"
"$drive" key enter

at done
click_at 72 699

at recap
"$drive" key escape
