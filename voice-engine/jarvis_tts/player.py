"""Continuous audio output: one output stream for the whole session and a
sample queue. Fragments are appended back to back, so there is no device
restart, file or process between them. Underruns (queue empty while the
utterance is still being generated) are measured, not hidden.
"""
from __future__ import annotations

import collections
import threading
import time

import numpy as np


class Player:
    def __init__(
        self,
        sample_rate: int,
        device_output: bool = True,
        gain: float = 1.0,
        blocksize: int = 512,
        prebuffer_ms: int = 350,
        rebuffer_ms: int = 200,
    ):
        self.sr = sample_rate
        self.gain = gain  # 0.0 = real device timing without audible sound (measurements)
        self.blocksize = blocksize
        # Playback of an utterance starts once this much audio is queued (or
        # generation ended), and resumes after an underrun with a smaller
        # cushion: early chunks are the slowest to generate, so a short
        # initial buffer avoids stuttering at the cost of a little latency.
        self.prebuffer = int(sample_rate * prebuffer_ms / 1000)
        self.rebuffer = int(sample_rate * rebuffer_ms / 1000)
        self._waiting_for = self.prebuffer
        self._lock = threading.Lock()
        self._queue: collections.deque[np.ndarray] = collections.deque()
        self._offset = 0
        self.utterance: int | None = None
        self.generating = False
        self.started_at: float | None = None  # first non-silent block sent to the device
        self.underrun_s = 0.0
        self.underruns = 0
        self._in_underrun = False
        self.played_s = 0.0
        self._stream = None
        self._sim = None
        if device_output:
            import sounddevice as sd

            self._stream = sd.OutputStream(samplerate=sample_rate, channels=1, dtype="float32", blocksize=blocksize, callback=self._callback)
            self._stream.start()
        else:
            self._sim = threading.Thread(target=self._simulate, daemon=True)
            self._sim.start()

    # -- control (any thread) --
    def begin(self, utterance: int) -> None:
        with self._lock:
            self._queue.clear()
            self._offset = 0
            self.utterance = utterance
            self.generating = True
            self.started_at = None
            self.underrun_s = 0.0
            self.underruns = 0
            self._in_underrun = False
            self.played_s = 0.0
            self._waiting_for = self.prebuffer

    def enqueue(self, utterance: int, audio: np.ndarray) -> None:
        with self._lock:
            if utterance == self.utterance and audio.size:
                self._queue.append(audio.astype(np.float32, copy=False))

    def end_of_input(self, utterance: int) -> None:
        with self._lock:
            if utterance == self.utterance:
                self.generating = False

    def stop(self) -> None:
        """Drops everything queued; the next device block is silence."""
        with self._lock:
            self._queue.clear()
            self._offset = 0
            self.utterance = None
            self.generating = False

    def drained(self) -> bool:
        with self._lock:
            return not self._queue and not self.generating

    def close(self) -> None:
        if self._stream is not None:
            self._stream.stop()
            self._stream.close()

    # -- audio thread --
    def _queued(self) -> int:
        return sum(c.size for c in self._queue) - self._offset

    def _fill(self, frames: int) -> np.ndarray:
        out = np.zeros(frames, dtype=np.float32)
        with self._lock:
            if self._waiting_for and self.generating and self._queued() < self._waiting_for:
                # Still buffering: count it as underrun only once playback has begun.
                if self.started_at is not None and self.utterance is not None:
                    self.underrun_s += frames / self.sr
                    if not self._in_underrun:
                        self.underruns += 1
                    self._in_underrun = True
                return out
            self._waiting_for = 0
            pos = 0
            while pos < frames and self._queue:
                chunk = self._queue[0]
                take = min(frames - pos, chunk.size - self._offset)
                out[pos : pos + take] = chunk[self._offset : self._offset + take]
                pos += take
                self._offset += take
                if self._offset >= chunk.size:
                    self._queue.popleft()
                    self._offset = 0
            if pos and self.started_at is None:
                self.started_at = time.perf_counter()
            if self.started_at is not None and self.utterance is not None:
                self.played_s += pos / self.sr
                if pos < frames and self.generating:
                    gap = (frames - pos) / self.sr
                    self.underrun_s += gap
                    if not self._in_underrun:
                        self.underruns += 1
                    self._in_underrun = True
                    self._waiting_for = self.rebuffer
                elif pos == frames:
                    self._in_underrun = False
        return out * self.gain

    def _callback(self, outdata, frames, _time, _status):
        outdata[:, 0] = self._fill(frames)

    def _simulate(self) -> None:
        period = self.blocksize / self.sr
        while True:
            self._fill(self.blocksize)
            time.sleep(period)
