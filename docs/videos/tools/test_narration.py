"""Tests for narration.py: uv run --python 3.12 --with pytest pytest docs/videos/tools"""

import pytest

from narration import (
    Cue,
    Mark,
    Pause,
    ScriptError,
    Speech,
    Word,
    build_cues,
    check_marks_reported,
    format_srt,
    format_timings,
    parse_lexicon,
    parse_script,
    speech_ssml,
    spoken_text,
    to_ssml,
    unspeak,
)

LEXICON = {"Rubrique": "roo-breek", "rig.yaml": "rig dot yammel", "YAML": "yammel"}


def test_script_reads_marks_pauses_and_speech_in_order():
    source = "# comment\n[mark hook]\nTwo agents.\n  One   paragraph. [pause 600]\n[mark next] Done."

    assert parse_script(source) == [
        Mark("hook"),
        Speech("Two agents. One paragraph."),
        Pause(600),
        Mark("next"),
        Speech("Done."),
    ]


@pytest.mark.parametrize(
    "source",
    ["[mark Hook]", "[mark a b]", "[pause 0]", "[pause 9000]", "[pause soon]", "[shout loud]", "[mark a] [mark a]"],
)
def test_script_refuses_bad_tokens(source):
    with pytest.raises(ScriptError):
        parse_script(source)


def test_lexicon_skips_comments_and_refuses_lines_without_a_tab():
    assert parse_lexicon("# note\n\nRubrique\troo-breek\n") == {"Rubrique": "roo-breek"}
    with pytest.raises(ScriptError):
        parse_lexicon("Rubrique roo-breek\n")


def test_ssml_escapes_text_and_substitutes_whole_words_longest_first():
    text = "Open rig.yaml & YAML in Rubrique, not Rubriques."

    assert speech_ssml(text, LEXICON) == (
        'Open <sub alias="rig dot yammel">rig.yaml</sub> &amp; <sub alias="yammel">YAML</sub> '
        'in <sub alias="roo-breek">Rubrique</sub>, not Rubriques.'
    )


def test_spoken_text_replaces_written_forms_for_plain_voices():
    assert spoken_text("Rubrique opens rig.yaml.", LEXICON) == "roo-breek opens rig dot yammel."


def test_ssml_document_has_voice_rate_bookmarks_and_breaks():
    ssml = to_ssml([Mark("hook"), Speech("Hi."), Pause(300)], {}, "en-US-AvaMultilingualNeural", "-5%")

    assert '<voice name="en-US-AvaMultilingualNeural"><prosody rate="-5%">' in ssml
    assert '<bookmark mark="hook"/>\nHi.\n<break time="300ms"/>' in ssml


def test_cues_end_at_sentences_long_gaps_and_length():
    words = [
        Word("Two", 0.0, 0.2),
        Word("agents", 0.2, 0.6),
        Word(".", 0.6, 0.6, punctuation=True),
        Word("One", 0.7, 0.9),
        Word("paragraph", 2.0, 2.5),
        Word("averyveryverylongwordthatfillsthecue", 2.5, 3.0),
    ]

    assert build_cues(words, max_chars=20) == [
        Cue(0.0, 0.6, "Two agents."),
        Cue(0.7, 0.9, "One"),
        Cue(2.0, 2.5, "paragraph"),
        Cue(2.5, 3.0, "averyveryverylongwordthatfillsthecue"),
    ]


def test_captions_show_the_written_form():
    cues = [Cue(0.0, 1.0, "Open rig dot yammel in roo-breek")]

    assert unspeak(cues, LEXICON) == [Cue(0.0, 1.0, "Open rig.yaml in Rubrique")]


def test_srt_and_timings_format():
    assert format_srt([Cue(1.5, 3661.25, "Hi.")]) == "1\n00:00:01,500 --> 01:01:01,250\nHi.\n"
    assert format_timings([("hook", 0.0), ("next", 1.23456)]) == "hook\t0.000\nnext\t1.235\n"


def test_missing_mark_from_the_voice_fails():
    parts = [Mark("hook"), Speech("Hi."), Mark("next")]

    check_marks_reported(parts, [("hook", 0.0), ("next", 1.0)])
    with pytest.raises(ScriptError, match="next"):
        check_marks_reported(parts, [("hook", 0.0)])
