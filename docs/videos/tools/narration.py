"""Turn an episode's narration script into SSML, captions and mark timings.

A script is plain text with two kinds of tokens:

- ``[mark NAME]``: a point the screen recording syncs to.
- ``[pause MS]``: silence, in milliseconds.

Lines that start with ``#`` are comments. Everything else is spoken.
"""

import re
from dataclasses import dataclass
from xml.sax.saxutils import escape, quoteattr

IPA_PREFIX = "ipa:"
MARK_NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
TOKEN = re.compile(r"\[(mark|pause)\s+([^\]]*)\]")
LONGEST_PAUSE_MS = 5000
SENTENCE_END = (".", "!", "?")


@dataclass(frozen=True)
class Mark:
    """A sync point named in the script."""

    name: str


@dataclass(frozen=True)
class Pause:
    """Silence of a fixed length."""

    milliseconds: int


@dataclass(frozen=True)
class Speech:
    """Text to speak, with whitespace collapsed."""

    text: str


Part = Mark | Pause | Speech


@dataclass(frozen=True)
class Word:
    """One word or punctuation mark the voice spoke, with its time in seconds."""

    text: str
    start: float
    end: float
    punctuation: bool = False


@dataclass(frozen=True)
class Cue:
    """One caption: text shown from start to end, in seconds."""

    start: float
    end: float
    text: str


class ScriptError(ValueError):
    """The script or the lexicon has a line that cannot be read."""


def parse_script(source: str) -> list[Part]:
    """Read a narration script into marks, pauses and speech, in order."""
    lines = [line for line in source.splitlines() if not line.lstrip().startswith("#")]
    text = " ".join(lines)
    parts: list[Part] = []
    position = 0
    for match in TOKEN.finditer(text):
        _append_speech(parts, text[position : match.start()])
        parts.append(_parse_token(match.group(1), match.group(2).strip()))
        position = match.end()
    _append_speech(parts, text[position:])
    _check_unique_marks(parts)
    return parts


def _append_speech(parts: list[Part], text: str) -> None:
    collapsed = " ".join(text.split())
    if not collapsed:
        return
    if "[" in collapsed or "]" in collapsed:
        raise ScriptError(f"unknown token near: {collapsed[:60]}")
    parts.append(Speech(collapsed))


def _parse_token(kind: str, value: str) -> Part:
    if kind == "mark":
        if not MARK_NAME.fullmatch(value):
            raise ScriptError(f"mark names use lowercase letters, digits and '-': {value!r}")
        return Mark(value)
    if not value.isdigit() or not 0 < int(value) <= LONGEST_PAUSE_MS:
        raise ScriptError(f"pause takes 1 to {LONGEST_PAUSE_MS} milliseconds: {value!r}")
    return Pause(int(value))


def _check_unique_marks(parts: list[Part]) -> None:
    seen: set[str] = set()
    for part in parts:
        if isinstance(part, Mark):
            if part.name in seen:
                raise ScriptError(f"mark {part.name!r} appears twice")
            seen.add(part.name)


def parse_lexicon(source: str) -> dict[str, str]:
    """Read ``written<TAB>spoken`` lines into a mapping. ``#`` starts a comment line.

    ``spoken`` is either how to say the word ("tee mux") or ``ipa:SOUNDS``
    ("ipa:ʁy.bʁik"), its pronunciation in the International Phonetic Alphabet.
    """
    lexicon: dict[str, str] = {}
    for number, line in enumerate(source.splitlines(), start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        written, separator, spoken = line.partition("\t")
        if not separator or not written.strip() or not spoken.strip():
            raise ScriptError(f"lexicon line {number} needs 'written<TAB>spoken': {line!r}")
        lexicon[written.strip()] = spoken.strip()
    return lexicon


def _lexicon_pattern(lexicon: dict[str, str]) -> re.Pattern[str] | None:
    """One pattern for every written form, longest first, matched as whole words."""
    if not lexicon:
        return None
    alternatives = "|".join(re.escape(written) for written in sorted(lexicon, key=len, reverse=True))
    return re.compile(rf"(?<![\w.])(?:{alternatives})(?!\w)")


def speech_ssml(text: str, lexicon: dict[str, str]) -> str:
    """Escape ``text`` for SSML and wrap each lexicon word in ``<sub>`` or ``<phoneme>``."""
    pattern = _lexicon_pattern(lexicon)
    if pattern is None:
        return escape(text)
    pieces: list[str] = []
    position = 0
    for match in pattern.finditer(text):
        pieces.append(escape(text[position : match.start()]))
        written = match.group(0)
        pieces.append(_word_ssml(written, lexicon[written]))
        position = match.end()
    pieces.append(escape(text[position:]))
    return "".join(pieces)


def _word_ssml(written: str, spoken: str) -> str:
    if spoken.startswith(IPA_PREFIX):
        sounds = spoken.removeprefix(IPA_PREFIX)
        return f'<phoneme alphabet="ipa" ph={quoteattr(sounds)}>{escape(written)}</phoneme>'
    return f"<sub alias={quoteattr(spoken)}>{escape(written)}</sub>"


def _aliases(lexicon: dict[str, str]) -> dict[str, str]:
    """The lexicon words said with another spelling (not given as IPA sounds)."""
    return {written: spoken for written, spoken in lexicon.items() if not spoken.startswith(IPA_PREFIX)}


def spoken_text(text: str, lexicon: dict[str, str]) -> str:
    """Replace each lexicon word with how it is said, for voices without SSML."""
    aliases = _aliases(lexicon)
    pattern = _lexicon_pattern(aliases)
    if pattern is None:
        return text
    return pattern.sub(lambda match: aliases[match.group(0)], text)


def to_ssml(parts: list[Part], lexicon: dict[str, str], voice: str, rate: str) -> str:
    """Build the SSML document Azure reads, one bookmark per mark."""
    body: list[str] = []
    for part in parts:
        if isinstance(part, Mark):
            body.append(f"<bookmark mark={quoteattr(part.name)}/>")
        elif isinstance(part, Pause):
            body.append(f'<break time="{part.milliseconds}ms"/>')
        else:
            body.append(speech_ssml(part.text, lexicon))
    return (
        '<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xml:lang="en-US">'
        f"<voice name={quoteattr(voice)}><prosody rate={quoteattr(rate)}>"
        + "\n".join(body)
        + "</prosody></voice></speak>\n"
    )


def repair_azure_words(words: list[Word], lexicon: dict[str, str]) -> list[Word]:
    """Fix the words Azure reports around a ``<sub>`` substitution.

    Azure reports the alias words ("rig", "dot", "yammel"), then one word that
    joins the end of the tag, the written form and the next real word:
    '">rig.yaml opens'. Put the written form back in place of the alias words,
    so a caption never splits it, and keep the real word after it.
    """
    written_forms = sorted(lexicon, key=len, reverse=True)
    repaired: list[Word] = []
    for word in words:
        if not word.text.startswith('">'):
            repaired.append(word)
            continue
        rest = word.text[2:]
        written = next((form for form in written_forms if rest.startswith(form)), None)
        if written is not None:
            rest = rest[len(written) :]
            _merge_alias(repaired, written, lexicon[written])
        if rest.strip():
            repaired.append(Word(rest.strip(), word.start, word.end, word.punctuation))
    return repaired


def _merge_alias(repaired: list[Word], written: str, alias: str) -> None:
    """Replace the alias words at the end of ``repaired`` with one written word."""
    count = len(alias.split())
    tail = repaired[-count:]
    if len(tail) != count or " ".join(word.text for word in tail) != alias:
        return
    del repaired[-count:]
    repaired.append(Word(written, tail[0].start, tail[-1].end))


def build_cues(words: list[Word], max_chars: int = 42, max_gap: float = 0.6) -> list[Cue]:
    """Group spoken words into short caption lines.

    A cue ends at the end of a sentence, before a silence longer than
    ``max_gap`` seconds, or before it would grow past ``max_chars``.
    """
    cues: list[Cue] = []
    current: list[Word] = []

    def close() -> None:
        if current:
            text = "".join(_joined(word, index) for index, word in enumerate(current))
            cues.append(Cue(current[0].start, current[-1].end, text))
            current.clear()

    for word in words:
        if word.punctuation:
            if current:
                current.append(word)
                if word.text.endswith(SENTENCE_END):
                    close()
            continue
        if current and _starts_new_cue(current, word, max_chars, max_gap):
            close()
        current.append(word)
    close()
    return cues


def _joined(word: Word, index: int) -> str:
    return word.text if word.punctuation or index == 0 else " " + word.text


def _starts_new_cue(current: list[Word], word: Word, max_chars: int, max_gap: float) -> bool:
    length = sum(len(_joined(item, index)) for index, item in enumerate(current))
    return word.start - current[-1].end > max_gap or length + 1 + len(word.text) > max_chars


def unspeak(cues: list[Cue], lexicon: dict[str, str]) -> list[Cue]:
    """Show lexicon words as written in captions, if the voice reported the spoken form."""
    spoken = {said: written for written, said in _aliases(lexicon).items()}
    pattern = _lexicon_pattern(spoken)
    if pattern is None:
        return cues
    return [Cue(cue.start, cue.end, pattern.sub(lambda match: spoken[match.group(0)], cue.text)) for cue in cues]


def _srt_time(seconds: float) -> str:
    milliseconds = round(seconds * 1000)
    hours, rest = divmod(milliseconds, 3_600_000)
    minutes, rest = divmod(rest, 60_000)
    whole, fraction = divmod(rest, 1000)
    return f"{hours:02}:{minutes:02}:{whole:02},{fraction:03}"


def format_srt(cues: list[Cue]) -> str:
    """Write cues as a SubRip (.srt) caption file."""
    blocks = [
        f"{number}\n{_srt_time(cue.start)} --> {_srt_time(cue.end)}\n{cue.text}\n"
        for number, cue in enumerate(cues, start=1)
    ]
    return "\n".join(blocks)


def format_timings(marks: list[tuple[str, float]]) -> str:
    """Write ``name<TAB>seconds`` lines, the format record.zsh reads."""
    return "".join(f"{name}\t{seconds:.3f}\n" for name, seconds in marks)


def check_marks_reported(parts: list[Part], marks: list[tuple[str, float]]) -> None:
    """Fail when the voice skipped a mark, so a recording never waits on a missing time."""
    expected = [part.name for part in parts if isinstance(part, Mark)]
    reported = [name for name, _ in marks]
    if reported != expected:
        missing = sorted(set(expected) - set(reported))
        raise ScriptError(f"the voice reported marks {reported}, expected {expected} (missing {missing})")
