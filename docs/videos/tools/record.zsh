#!/bin/zsh
# Record one episode: drive a sample window to the narration's marks while
# the screen records, then add the voice and captions.
#
#   docs/videos/tools/record.zsh target/debug/rubrique docs/videos/episodes/21-compare
#
# Run synth.py for the episode first. The terminal needs Screen Recording
# and Accessibility access, and ffmpeg must be installed. Leave the
# keyboard and mouse idle while it runs.
#
# The episode's steps.zsh sets up and starts the window, calls
# start_recording, then waits for each mark with `at NAME` before a step.
# Steps point with `glide_to X Y [MS]` and `click_at X Y`, in points of the
# recorded picture (a frame of the video at half size), so read them off a
# frame of an earlier take.
# PROBE=1 runs the steps without voice or video: each `at` waits a moment
# and saves a screenshot to out/EPISODE/probe/, to find click positions.
# CAPTURE_LATENCY (seconds, default 0.3) is how long screencapture takes to
# start; raise it if the voice runs ahead of the screen.
# Stop at the first failing command (see capture.zsh for why not `set -e`).
set -u
trap 'exit 1' ZERR
zmodload zsh/datetime zsh/mathfunc
episode=$(cd "${2:?pass the episode folder}" && pwd)
videos=$(cd "$(dirname "$0")/.." && pwd)
out="$videos/out/${episode:t}"
probe=${PROBE:-0}
for file in voice.wav timings.tsv; do
  if [[ $probe != 1 && ! -f "$out/$file" ]]; then
    echo "Run synth.py for ${episode:t} first: $out/$file is missing." >&2
    exit 1
  fi
done
if ! command -v ffmpeg >/dev/null; then
  echo "Install ffmpeg first (brew install ffmpeg)." >&2
  exit 1
fi

# 1280 x 720 points is 2560 x 1440 pixels on a 2x display: 16:9 for YouTube.
window_width=1280
window_height=720
text_scale=${text_scale:-1.1}
# A software engineer's project: a spec and the build plan for an agent team.
sample_source="$videos/sample/invoice-export"
sample_path=/tmp/invoice-export
sample_tabs="spec.md build-plan.md"
source "$videos/../screenshots/tools/capture.zsh" "$1"

typeset -A mark_times
voice_seconds=0
if [[ $probe != 1 ]]; then
  while IFS=$'\t' read -r name seconds; do
    mark_times[$name]=$seconds
  done < "$out/timings.tsv"
  voice_seconds=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$out/voice.wav")
fi
probe_shots=0
lead_in=1.5
tail_out=2
capture_latency=${CAPTURE_LATENCY:-0.3}
recorder=""
started=""
title_bar=0

stop_recorder() {
  if [[ -n "$recorder" ]]; then
    kill "$recorder" 2>/dev/null || true
    wait "$recorder" 2>/dev/null || true
    recorder=""
  fi
}
trap 'stop_recorder; cleanup' EXIT

# Record the window's content (its bottom window_height points, below the
# title bar) for the length of the voice plus a lead-in and a tail.
start_recording() {
  local x y width height
  require_front
  started=$EPOCHREALTIME
  if [[ $probe == 1 ]]; then
    read -r x y width height <<< "$("$drive" frame "$id")"
    title_bar=$(( height - window_height ))
    mkdir -p "$out/probe"
    rm -f "$out/probe/"*.png(N)
    return
  fi
  read -r x y width height <<< "$("$drive" frame "$id")"
  title_bar=$(( height - window_height ))
  local top=$(( y + title_bar ))
  local seconds=$(( int(ceil(voice_seconds + lead_in + tail_out)) ))
  # screencapture refuses to replace an earlier take.
  rm -f "$out/screen.mov"
  screencapture -v -V "$seconds" -k -C -x -R "$x,$top,$window_width,$window_height" "$out/screen.mov" &
  recorder=$!
}

# Stop the run unless the sample window's app is in front, so clicks and
# typing never land in another app (for example when someone uses the Mac).
require_front() {
  "$drive" front "$pid"
}

# Move the pointer smoothly to (X, Y) of the recorded picture.
glide_to() {
  require_front
  "$drive" glide "$id" "$1" $(( $2 + title_bar )) ${3:+"$3"}
}

# Glide to (X, Y) of the recorded picture and click there.
click_at() {
  glide_to "$1" "$2"
  "$drive" click "$id" "$1" $(( $2 + title_bar ))
}

# Wait until the voice reaches MARK, so the next step happens as it is said.
at() {
  local mark="$1"
  if [[ $probe == 1 ]]; then
    sleep 1.5
    require_front
    probe_shots=$(( probe_shots + 1 ))
    screencapture -x -o -l "$id" "$work/probe.png"
    swift "$tools/paste.swift" "$out/probe/$(printf %02d "$probe_shots")-$mark.png" "$work/probe.png" \
      "$window_width" "$window_height"
    return
  fi
  if [[ -z "${mark_times[$mark]:-}" ]]; then
    echo "The script has no mark '$mark'." >&2
    exit 1
  fi
  local remaining=$(( started + lead_in + ${mark_times[$mark]} - EPOCHREALTIME ))
  if (( remaining > 0 )); then
    sleep "$remaining"
  else
    printf 'The step at %s started %.2f seconds late.\n' "$mark" $(( -remaining )) >&2
  fi
  require_front
}

# Wait for the recording, then add the voice and shift the captions to match.
finish_recording() {
  wait "$recorder"
  recorder=""
  local delay=$(( lead_in - capture_latency ))
  local name="${episode:t}"
  ffmpeg -loglevel error -y -i "$out/screen.mov" -i "$out/voice.wav" \
    -filter:a "adelay=$(( int(delay * 1000) )):all=1" -map 0:v -map 1:a \
    -c:v libx264 -crf 18 -preset slow -pix_fmt yuv420p -r 30 \
    -c:a aac -b:a 192k -movflags +faststart "$out/$name.mp4"
  if [[ -f "$out/captions.srt" ]]; then
    ffmpeg -loglevel error -y -itsoffset "$delay" -i "$out/captions.srt" "$out/$name.srt"
  fi
  echo "Wrote $out/$name.mp4"
}

"$drive" permissions input
source "$episode/steps.zsh"
if [[ -z "$started" ]]; then
  echo "${episode:t}/steps.zsh must call start_recording." >&2
  exit 1
fi
if [[ $probe == 1 ]]; then
  echo "Wrote $out/probe/"
else
  finish_recording
fi
