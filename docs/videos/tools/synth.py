# /// script
# requires-python = ">=3.11"
# dependencies = ["azure-cognitiveservices-speech==1.53.0"]
# ///
"""Speak an episode's narration and write the files record.zsh needs.

    uv run docs/videos/tools/synth.py docs/videos/episodes/21-compare
    uv run docs/videos/tools/synth.py --draft docs/videos/episodes/21-compare
    uv run docs/videos/tools/synth.py --ssml docs/videos/episodes/21-compare

Writes to docs/videos/out/EPISODE/: voice.wav, timings.tsv (one line per
mark) and, with Azure, captions.srt. Azure reads AZURE_SPEECH_KEY and
AZURE_SPEECH_REGION from the environment. --draft uses the macOS `say`
voice instead, free and offline, to rehearse the timing. --ssml only
prints the SSML and the character count Azure would bill.
"""

import argparse
import logging
import os
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

from narration import (
    Mark,
    Part,
    Pause,
    Word,
    build_cues,
    check_marks_reported,
    format_srt,
    format_timings,
    parse_lexicon,
    parse_script,
    repair_azure_words,
    spoken_text,
    to_ssml,
    unspeak,
)

VIDEOS = Path(__file__).resolve().parent.parent
DEFAULT_VOICE = "en-US-AndrewMultilingualNeural"
DEFAULT_RATE = "-5%"
SAMPLE_RATE = 48_000
# Azure reports audio offsets in ticks of 100 nanoseconds.
TICKS_PER_SECOND = 10_000_000

log = logging.getLogger("synth")


def synthesize_azure(ssml: str, wav: Path) -> tuple[list[tuple[str, float]], list[Word]]:
    """Speak ``ssml`` with Azure into ``wav``; return the mark times and spoken words."""
    import azure.cognitiveservices.speech as speechsdk  # noqa: PLC0415 (only the Azure path needs the SDK)

    key, region = os.environ.get("AZURE_SPEECH_KEY"), os.environ.get("AZURE_SPEECH_REGION")
    if not key or not region:
        raise SystemExit("Set AZURE_SPEECH_KEY and AZURE_SPEECH_REGION, or use --draft.")
    config = speechsdk.SpeechConfig(subscription=key, region=region)
    config.set_speech_synthesis_output_format(speechsdk.SpeechSynthesisOutputFormat.Riff48Khz16BitMonoPcm)
    synthesizer = speechsdk.SpeechSynthesizer(
        speech_config=config, audio_config=speechsdk.audio.AudioOutputConfig(filename=str(wav))
    )
    marks: list[tuple[str, float]] = []
    words: list[Word] = []
    kinds = {
        speechsdk.SpeechSynthesisBoundaryType.Word: False,
        speechsdk.SpeechSynthesisBoundaryType.Punctuation: True,
    }

    def on_word(event: speechsdk.SpeechSynthesisWordBoundaryEventArgs) -> None:
        if event.boundary_type not in kinds:
            return
        start = event.audio_offset / TICKS_PER_SECOND
        words.append(Word(event.text, start, start + event.duration.total_seconds(), kinds[event.boundary_type]))

    synthesizer.bookmark_reached.connect(
        lambda event: marks.append((event.text, event.audio_offset / TICKS_PER_SECOND))
    )
    synthesizer.synthesis_word_boundary.connect(on_word)
    result = synthesizer.speak_ssml_async(ssml).get()
    if result.reason != speechsdk.ResultReason.SynthesizingAudioCompleted:
        details = result.cancellation_details
        raise SystemExit(f"Azure did not finish: {details.reason} {details.error_details}")
    return marks, words


def synthesize_draft(parts: list[Part], lexicon: dict[str, str], wav: Path) -> list[tuple[str, float]]:
    """Speak each part with macOS `say` into one ``wav``; return the mark times."""
    marks: list[tuple[str, float]] = []
    frames = bytearray()
    bytes_per_second = SAMPLE_RATE * 2
    with tempfile.TemporaryDirectory() as temporary:
        text_file, part_wav = Path(temporary) / "part.txt", Path(temporary) / "part.wav"
        for part in parts:
            if isinstance(part, Mark):
                marks.append((part.name, len(frames) / bytes_per_second))
            elif isinstance(part, Pause):
                frames.extend(bytes(bytes_per_second * part.milliseconds // 1000 // 2 * 2))
            else:
                text_file.write_text(spoken_text(part.text, lexicon))
                say = ["say", "-f", str(text_file), "-o", str(part_wav)]
                # A fixed argument list; the narration goes through a file, never the command line.
                subprocess.run([*say, "--file-format=WAVE", f"--data-format=LEI16@{SAMPLE_RATE}"], check=True)  # noqa: S603
                with wave.open(str(part_wav), "rb") as spoken:
                    frames.extend(spoken.readframes(spoken.getnframes()))
    with wave.open(str(wav), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(SAMPLE_RATE)
        out.writeframes(bytes(frames))
    return marks


def main() -> None:
    """Read the episode, speak it and write voice.wav, timings.tsv and captions.srt."""
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    parser = argparse.ArgumentParser(description="Speak an episode's narration script.")
    parser.add_argument("episode", type=Path, help="episode folder with script.txt")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--draft", action="store_true", help="use the macOS say voice (free, offline)")
    mode.add_argument("--ssml", action="store_true", help="print the SSML and character count, speak nothing")
    parser.add_argument("--voice", default=DEFAULT_VOICE, help=f"Azure voice (default {DEFAULT_VOICE})")
    parser.add_argument("--rate", default=DEFAULT_RATE, help=f"speaking rate (default {DEFAULT_RATE})")
    args = parser.parse_args()

    script = args.episode / "script.txt"
    if not script.is_file():
        raise SystemExit(f"No narration script at {script}")
    parts = parse_script(script.read_text())
    lexicon = parse_lexicon((VIDEOS / "lexicon.tsv").read_text())
    ssml = to_ssml(parts, lexicon, args.voice, args.rate)
    if args.ssml:
        sys.stdout.write(ssml)
        log.info("%d characters", len(ssml))
        return

    out = VIDEOS / "out" / args.episode.resolve().name
    out.mkdir(parents=True, exist_ok=True)
    wav = out / "voice.wav"
    if args.draft:
        marks = synthesize_draft(parts, lexicon, wav)
        (out / "captions.srt").unlink(missing_ok=True)
    else:
        marks, words = synthesize_azure(ssml, wav)
        cues = build_cues(repair_azure_words(words, lexicon))
        (out / "captions.srt").write_text(format_srt(unspeak(cues, lexicon)))
    check_marks_reported(parts, marks)
    (out / "timings.tsv").write_text(format_timings(marks))
    log.info("Wrote %s (%d marks)", out, len(marks))


if __name__ == "__main__":
    main()
