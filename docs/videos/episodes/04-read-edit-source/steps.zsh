# Episode 04: Read, Edit and Source. Sourced by record.zsh.
start_window dark spec.md false
"$drive" activate "$pid"
sleep 0.5
start_recording
glide_to 1000 600 10

at read
glide_to 760 300

at edit
"$drive" key cmd+shift+e

# The first paragraph under "Before the build". The block editor opens with
# the cursor at the start; a click past the end of its last line moves it
# to the end.
at block
click_at 1192 426
sleep 0.8
click_at 700 458

at type
"$drive" type " Log every export with the caller ID."

at done
"$drive" key escape

at insert
glide_to 783 562
sleep 0.6
click_at 783 562

at insert-type
"$drive" type "Return 404 for invoices of other accounts."

at insert-done
"$drive" key escape

at undo
"$drive" key cmd+z

at source
"$drive" key cmd+e

at recap
