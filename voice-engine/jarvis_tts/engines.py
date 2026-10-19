"""Neural TTS engines (MLX, local). Each engine is loaded once and reused.

Engine.stream(fragment, cancel) yields float32 mono numpy arrays at
`sample_rate`. Cancellation is checked between yields; MLX cannot interrupt
a single forward pass, so the worst-case stop latency for generation is one
streaming step (playback stops immediately in the player).
"""
from __future__ import annotations

import threading
from pathlib import Path
from typing import Iterator, Protocol

import numpy as np

ENGINE_IDS = ("qwen3-clone", "qwen3-design", "chatterbox-latam")


class Engine(Protocol):
    id: str
    sample_rate: int

    def stream(self, text: str, cancel: threading.Event) -> Iterator[np.ndarray]: ...


def _np(a) -> np.ndarray:
    return np.asarray(a, dtype=np.float32).reshape(-1)


class Qwen3Clone:
    """Qwen3-TTS Base: clones a fixed reference voice for consistent timbre."""

    id = "qwen3-clone"

    def __init__(self, repo: str, ref_audio: Path, ref_text: str, streaming_interval: float = 0.48):
        from mlx_audio.tts.utils import load_model

        self.model = load_model(repo)
        self.sample_rate = self.model.sample_rate
        self.ref_audio = str(ref_audio)
        self.ref_text = ref_text
        self.interval = streaming_interval

    def stream(self, text: str, cancel: threading.Event) -> Iterator[np.ndarray]:
        for r in self.model.generate(
            text=text,
            ref_audio=self.ref_audio,
            ref_text=self.ref_text,
            lang_code="spanish",
            stream=True,
            streaming_interval=self.interval,
            split_pattern="\x00",
        ):
            if cancel.is_set():
                return
            yield _np(r.audio)


class Qwen3Design:
    """Qwen3-TTS VoiceDesign: voice from a text description (may vary per call)."""

    id = "qwen3-design"

    def __init__(self, repo: str, instruct: str, streaming_interval: float = 0.48):
        from mlx_audio.tts.utils import load_model

        self.model = load_model(repo)
        self.sample_rate = self.model.sample_rate
        self.instruct = instruct
        self.interval = streaming_interval

    def stream(self, text: str, cancel: threading.Event) -> Iterator[np.ndarray]:
        for r in self.model.generate_voice_design(
            text=text, instruct=self.instruct, language="Spanish", stream=True, streaming_interval=self.interval
        ):
            if cancel.is_set():
                return
            yield _np(r.audio)


class ChatterboxLatam:
    """Chatterbox Multilingual V3 + official es-mx-latam finetune (MLX build)."""

    id = "chatterbox-latam"

    def __init__(self, model_dir: Path, ref_audio: Path | None = None, exaggeration: float = 0.5):
        from mlx_audio.tts.utils import load_model

        self.model = load_model(str(model_dir))
        self.sample_rate = self.model.sample_rate
        self.ref_audio = str(ref_audio) if ref_audio else None
        self.exaggeration = exaggeration

    def stream(self, text: str, cancel: threading.Event) -> Iterator[np.ndarray]:
        for r in self.model.generate(
            text=text,
            lang_code="es",
            ref_audio=self.ref_audio,
            exaggeration=self.exaggeration,
            stream=True,
            streaming_interval=1.0,
            verbose=False,
        ):
            if cancel.is_set():
                return
            yield _np(r.audio)


# Absolute level below which a sample counts as silence (-50 dBFS). Absolute,
# so an all-silent chunk is not mistaken for speech.
SILENCE = 10 ** (-50 / 20)


def first_voiced(audio: np.ndarray) -> int | None:
    idx = np.flatnonzero(np.abs(audio) > SILENCE)
    return int(idx[0]) if idx.size else None


def last_voiced(audio: np.ndarray) -> int | None:
    idx = np.flatnonzero(np.abs(audio) > SILENCE)
    return int(idx[-1]) if idx.size else None


def trim_head(audio: np.ndarray, sr: int, keep_ms: int = 60) -> np.ndarray:
    """Drops leading silence beyond `keep_ms` (model padding)."""
    i = first_voiced(audio)
    return audio if i is None else audio[max(0, i - int(sr * keep_ms / 1000)) :]


def trim_tail(audio: np.ndarray, sr: int, keep_ms: int = 160) -> np.ndarray:
    """Drops trailing silence beyond `keep_ms`, leaving a natural pause."""
    i = last_voiced(audio)
    return audio if i is None else audio[: min(audio.size, i + 1 + int(sr * keep_ms / 1000))]
