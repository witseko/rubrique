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

## Episode folder

- `script.txt`: the narration. `[mark NAME]` is a point the recording
  syncs to; `[pause MS]` is silence; lines that start with `#` are notes.
- `steps.zsh`: sourced by `record.zsh`. It starts the sample window with
  the helpers in `docs/screenshots/tools/capture.zsh`, calls
  `start_recording`, then runs each step after `at NAME`. Window points
  count from the top left of the title bar.

`drive.swift` (in `docs/screenshots/tools/`) has `glide ID X Y [MS]` for
pointer moves a viewer can follow; use it before each `click`.

## Pronunciation

`lexicon.tsv` lists words the voice says wrong (`written<TAB>spoken`).
Every episode uses it, and the captions keep the written form.

## Tests

```sh
uv run --python 3.12 --with pytest pytest docs/videos/tools
```
