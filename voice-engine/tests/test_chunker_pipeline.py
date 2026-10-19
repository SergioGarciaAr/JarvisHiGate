"""Unit tests for fragmenting and edge trimming (no model needed)."""
import sys, threading
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from jarvis_tts.chunker import chunks, sentences  # noqa: E402
from jarvis_tts.engines import trim_head, trim_tail  # noqa: E402
from jarvis_tts.pipeline import Piece, realtime_render, stream_text, timeline  # noqa: E402

SR = 24000


def test_sentences_respect_abbreviations_and_lines():
    s = sentences("Hola, Sr. García. ¿Cómo está?\nTodo bien. Fin.")
    assert s == ["Hola, Sr. García.", "¿Cómo está?", "Todo bien.", "Fin."]


def test_first_chunk_short_then_grouped_without_splitting_sentences():
    text = " ".join(f"Esta es la frase número {i} de una explicación algo larga." for i in range(12))
    cs = chunks(text)
    assert len(cs) > 2
    assert len(cs[0]) <= 160
    for c in cs:
        assert c.endswith("."), c  # every fragment ends at a sentence boundary
    assert " ".join(cs) == text  # nothing lost or duplicated


def test_very_long_sentence_splits_at_punctuation_never_inside_words():
    long = "Primero revisé la memoria, " + ", ".join(f"luego el componente {i}" for i in range(40)) + "."
    cs = chunks(long)
    assert len(cs) > 1
    assert " ".join(cs).replace("  ", " ") == long
    for c in cs[:-1]:
        assert c.endswith(","), c


def test_trim_keeps_speech_and_removes_padding():
    speech = np.sin(np.linspace(0, 400, SR)).astype(np.float32) * 0.5
    padded = np.concatenate([np.zeros(SR), speech, np.zeros(SR)]).astype(np.float32)
    head = trim_head(padded, SR)
    lead = head.size - speech.size - SR  # remaining leading silence
    assert SR * 0.05 <= lead <= SR * 0.07, lead
    tail = trim_tail(head, SR)
    assert abs(tail.size - (lead + speech.size + int(SR * 0.16))) <= 2 * SR // 1000


def test_timeline_measures_underruns_and_renders_them():
    one = np.ones(SR, dtype=np.float32) * 0.1  # 1 s
    pieces = [Piece(one, 0, 0.2), Piece(one, 1, 1.0), Piece(one, 2, 2.6)]
    t = timeline(pieces, SR)
    assert abs(t["first_audio_s"] - 0.2) < 1e-9
    assert len(t["gaps"]) == 1 and abs(t["gaps"][0][1] - 0.4) < 1e-6
    assert realtime_render(pieces, SR).size == 3 * SR + int(0.4 * SR)


class FakeEngine:
    id = "fake"
    sample_rate = SR

    def __init__(self):
        self.calls = []

    def stream(self, text, cancel):
        self.calls.append(text)
        for _ in range(3):
            if cancel.is_set():
                return
            yield np.concatenate([np.zeros(SR // 4), np.ones(SR // 4) * 0.3, np.zeros(SR // 4)]).astype(np.float32)


def test_no_lookahead_first_piece_arrives_with_first_chunk_and_pauses_are_kept():
    class Paced:
        id, sample_rate = "paced", SR

        def stream(self, text, cancel):
            speech = (np.ones(SR // 10) * 0.3).astype(np.float32)
            yield np.concatenate([np.zeros(SR // 2, dtype=np.float32), speech, np.zeros(SR // 5, dtype=np.float32)])  # padding, word, pause
            yield np.concatenate([speech, np.zeros(SR, dtype=np.float32)])  # word, long end padding

    pieces = list(stream_text(Paced(), "Una frase.", threading.Event()))
    sizes = [p.audio.size for p in pieces]
    # 1st piece: 60 ms of head + first word, emitted before the second chunk is used.
    assert sizes[0] == int(SR * 0.06) + SR // 10
    # The 200 ms pause between words is preserved, the 1 s end padding capped to 160 ms.
    assert sizes[1] == SR // 5 + SR // 10
    assert sizes[2] == int(SR * 0.16)


def test_cancel_stops_generation_between_chunks():
    eng, cancel = FakeEngine(), threading.Event()
    text = "Uno. " * 200
    got = []
    for p in stream_text(eng, text, cancel):
        got.append(p)
        if len(got) == 2:
            cancel.set()
    assert len(got) == 2
    assert len(eng.calls) == 1  # no further fragments requested after cancel
