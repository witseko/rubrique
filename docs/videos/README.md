# Tutorial videos

Each episode is a short YouTube tutorial: a narration script spoken by an
Azure neural voice, and a screen recording of a sample window that the
tools drive in time with the voice. Re-render an episode after the UI
changes instead of recording it again by hand.

## Make an episode

Run these on a Mac with a 2x display, from a terminal with Screen
Recording and Accessibility access. Leave the keyboard and mouse idle
while `record.zsh` runs, and close other Rubrique windows first.

```sh
cargo build --locked
uv run docs/videos/tools/synth.py --draft docs/videos/episodes/21-compare
docs/videos/tools/record.zsh target/debug/rubrique docs/videos/episodes/21-compare
```

`--draft` speaks with the macOS `say` voice, free and offline, to rehearse
the timing. For the real voice, set `AZURE_SPEECH_KEY` and
`AZURE_SPEECH_REGION` and run `synth.py` without `--draft`; it also writes
captions. `--ssml` prints what Azure would read and the billed character
count.

`record.zsh` writes `docs/videos/out/EPISODE/EPISODE.mp4` (2560 x 1440,
H.264) and, with Azure, `EPISODE.srt` for YouTube's captions. If the voice
runs ahead of the screen, set `CAPTURE_LATENCY` (seconds, default 0.3).

## Sample project

Episodes show a software engineer's work: `sample/invoice-export/` holds a
spec and the build plan for an agent team, copied to `/tmp/invoice-export`
for each take. The stand-in `claude` and `codex` commands answer with the
files in `answers/`, so Ask and Compare give the same result every take.

## Episode folder

- `script.txt`: the narration. `[mark NAME]` is a point the recording
  syncs to; `[pause MS]` is silence; lines that start with `#` are notes.
- `steps.zsh`: sourced by `record.zsh`. It starts the sample window with
  the helpers in `docs/screenshots/tools/capture.zsh`, calls
  `start_recording`, then runs each step after `at NAME`. `click_at X Y`
  and `glide_to X Y` take points of the recorded picture.

To find those points, run `PROBE=1 docs/videos/tools/record.zsh ...`: it
runs the steps without voice or video and saves a screenshot at each mark
to `out/EPISODE/probe/` (2x pixels; halve them for points). A run stops
when another app comes to the front, so no input lands there.

`drive.swift` (in `docs/screenshots/tools/`) has `glide ID X Y [MS]` for
pointer moves a viewer can follow; use it before each `click`.

## Pronunciation

`lexicon.tsv` lists words the voice says wrong (`written<TAB>spoken`).
Every episode uses it, and the captions keep the written form.

## Tests

```sh
uv run --python 3.12 --with pytest pytest docs/videos/tools
```
