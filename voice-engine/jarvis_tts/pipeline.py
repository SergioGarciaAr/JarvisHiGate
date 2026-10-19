"""Text → continuous audio: fragments at sentence boundaries, streams each
fragment, and trims only the model's padding at fragment edges, so fragments
join without long artificial gaps.

No lookahead: each chunk is emitted as soon as it arrives except its
trailing silence, which is carried. If more speech follows, the carried
silence was a real pause and is emitted unchanged; at the end of a fragment
it is capped to TAIL_KEEP_MS."""
from __future__ import annotations

import threading
import time
from dataclasses import dataclass, field
from typing import Iterator

import numpy as np

from .chunker import chunks
from .engines import Engine, first_voiced, last_voiced

HEAD_KEEP_MS = 60
TAIL_KEEP_MS = 160


@dataclass
class Piece:
    audio: np.ndarray
    fragment: int
    ready_at: float  # seconds since synthesis started


@dataclass
class FragmentStats:
    text: str
    gen_s: float
    audio_s: float


@dataclass
class Synthesis:
    pieces: list[Piece] = field(default_factory=list)
    fragments: list[FragmentStats] = field(default_factory=list)


def stream_text(engine: Engine, text: str, cancel: threading.Event, t0: float | None = None) -> Iterator[Piece]:
    t0 = t0 or time.perf_counter()
    sr = engine.sample_rate
    head_keep = int(sr * HEAD_KEEP_MS / 1000)
    tail_keep = int(sr * TAIL_KEEP_MS / 1000)
    for i, frag in enumerate(chunks(text)):
        if cancel.is_set():
            return
        carry = np.zeros(0, dtype=np.float32)
        started = False
        for audio in engine.stream(frag, cancel):
            if cancel.is_set():
                return
            if not started:
                f = first_voiced(audio)
                if f is None:
                    continue  # leading padding only
                audio = audio[max(0, f - head_keep) :]
                started = True
            audio = np.concatenate([carry, audio]) if carry.size else audio
            last = last_voiced(audio)
            if last is None:
                carry = audio
                continue
            voiced, carry = audio[: last + 1], audio[last + 1 :]
            yield Piece(voiced, i, time.perf_counter() - t0)
        if carry.size and not cancel.is_set():
            yield Piece(carry[:tail_keep], i, time.perf_counter() - t0)


def timeline(pieces: list[Piece], sr: int) -> dict:
    """Simulates real-time playback: each piece starts when it is ready and
    the previous one has finished. Gaps are underruns the listener hears."""
    clock = None
    gaps = []
    for p in pieces:
        dur = p.audio.size / sr
        if clock is None:
            first_audio = p.ready_at
            clock = p.ready_at + dur
            continue
        if p.ready_at > clock:
            gaps.append((p.fragment, p.ready_at - clock))
            clock = p.ready_at
        clock += dur
    total_audio = sum(p.audio.size for p in pieces) / sr
    return {
        "first_audio_s": pieces[0].ready_at if pieces else None,
        "audio_s": total_audio,
        "gen_s": pieces[-1].ready_at if pieces else 0.0,
        "gaps": gaps,
        "gap_total_s": sum(g for _, g in gaps),
        "gaps_over_50ms": sum(1 for _, g in gaps if g > 0.05),
    }


def realtime_render(pieces: list[Piece], sr: int) -> np.ndarray:
    """Audio exactly as a listener would hear it in real time, including any
    underrun gaps as silence (not added for effect: they are measured)."""
    out: list[np.ndarray] = []
    clock = None
    for p in pieces:
        if clock is not None and p.ready_at > clock:
            out.append(np.zeros(round((p.ready_at - clock) * sr), dtype=np.float32))
            clock = p.ready_at
        clock = (clock if clock is not None else p.ready_at) + p.audio.size / sr
        out.append(p.audio)
    return np.concatenate(out) if out else np.zeros(0, dtype=np.float32)
