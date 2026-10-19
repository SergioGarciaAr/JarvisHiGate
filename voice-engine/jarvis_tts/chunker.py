"""Splits prepared text into synthesis fragments at natural boundaries.

Rules:
- Fragments end at the end of a complete sentence whenever possible.
- The first fragment is kept short (one sentence) so audio starts quickly;
  later fragments group sentences up to `max_chars` so the engine runs fewer,
  longer, more natural passes.
- A sentence longer than `hard_max` is split at `;`, `:` or `,` before the
  limit, and only as a last resort at a space. Never inside a word.
"""
import re

_SENTENCE_END = re.compile(r"(?<=[.!?…])[\"»)\]]*\s+(?=[¿¡\"«(\[]?[A-ZÁÉÍÓÚÑÜ0-9])")
_ABBREVIATIONS = ("Sr.", "Sra.", "Dr.", "Dra.", "Ud.", "Uds.", "etc.", "p. ej.", "aprox.", "núm.", "pág.")


def sentences(text: str) -> list[str]:
    out: list[str] = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = _SENTENCE_END.split(line)
        merged: list[str] = []
        for p in parts:
            if merged and merged[-1].endswith(_ABBREVIATIONS):
                merged[-1] = f"{merged[-1]} {p}"
            else:
                merged.append(p)
        out.extend(s.strip() for s in merged if s.strip())
    return out


def _split_long(sentence: str, hard_max: int) -> list[str]:
    pieces: list[str] = []
    rest = sentence
    while len(rest) > hard_max:
        window = rest[:hard_max]
        cut = max(window.rfind("; "), window.rfind(": "), window.rfind(", "))
        if cut < hard_max // 3:
            cut = window.rfind(" ")
        if cut <= 0:
            cut = hard_max
        pieces.append(rest[: cut + 1].strip())
        rest = rest[cut + 1 :].strip()
    if rest:
        pieces.append(rest)
    return pieces


def chunks(text: str, first_max: int = 160, max_chars: int = 320, hard_max: int = 400) -> list[str]:
    units: list[str] = []
    for s in sentences(text):
        units.extend(_split_long(s, hard_max) if len(s) > hard_max else [s])
    out: list[str] = []
    current = ""
    for u in units:
        limit = first_max if not out else max_chars
        if not current:
            current = u
        elif len(current) + 1 + len(u) <= limit:
            current = f"{current} {u}"
        else:
            out.append(current)
            current = u
    if current:
        out.append(current)
    return out
