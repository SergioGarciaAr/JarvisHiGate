"""Jarvis neural TTS sidecar. One long-lived process: the model is loaded
once, audio plays through one continuous output stream.

Protocol: JSON lines. stdin commands:
  {"cmd": "speak", "id": 7, "text": "..."}   replaces any current utterance
  {"cmd": "stop"}                            stops generation and playback
  {"cmd": "shutdown"}
stdout events:
  {"event": "ready", "engine": ..., "sample_rate": ..., "load_ms": ...}
  {"event": "first_audio", "id": 7, "ms": 812}     audio reached the device
  {"event": "finished", "id": 7, "underruns": 0, "underrun_ms": 0, "audio_ms": ...}
  {"event": "stopped", "id": 7}
  {"event": "error", "id": 7, "message": "..."}

Runs offline (HF_HUB_OFFLINE=1 is set by the caller); text is never sent anywhere.
"""
from __future__ import annotations

import argparse
import json
import os
import queue
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from jarvis_tts.pipeline import stream_text  # noqa: E402
from jarvis_tts.player import Player  # noqa: E402

_out_lock = threading.Lock()


def emit(**event) -> None:
    with _out_lock:
        sys.stdout.write(json.dumps(event, ensure_ascii=False) + "\n")
        sys.stdout.flush()


class Server:
    """All MLX work happens on one dedicated synthesis thread: MLX GPU streams
    are bound to the thread that creates them, so generating from a new
    thread per utterance fails with "There is no Stream(gpu, N) in current
    thread" while the previous one is still alive. Requests go through a
    queue; a new request cancels the current one and stale jobs are skipped."""

    def __init__(self, engine_factory, player_factory):
        self.current: int | None = None
        self.cancel = threading.Event()
        self.lock = threading.Lock()
        self.jobs: queue.Queue = queue.Queue()
        self.ready = threading.Event()
        self.startup_error: str | None = None
        self.engine = None
        self.player = None
        self._factories = (engine_factory, player_factory)
        threading.Thread(target=self._synthesis_thread, name="jarvis-tts-synthesis", daemon=True).start()

    def _synthesis_thread(self) -> None:
        engine_factory, player_factory = self._factories
        try:
            self.engine = engine_factory()
            for _ in stream_text(self.engine, "Listo.", threading.Event()):  # warm-up
                pass
            self.player = player_factory(self.engine.sample_rate)
        except Exception as e:
            self.startup_error = f"{type(e).__name__}: {e}"
            self.ready.set()
            return
        self.ready.set()
        while True:
            uid, text, cancel, t0 = self.jobs.get()
            if uid is None:
                return
            if cancel.is_set():
                continue  # superseded before it started
            self._generate(uid, text, cancel, t0)

    def speak(self, uid: int, text: str) -> None:
        with self.lock:
            self._stop_locked()
            self.current = uid
            self.cancel = threading.Event()
            self.player.begin(uid)
            t0 = time.perf_counter()
            self.jobs.put((uid, text, self.cancel, t0))
            threading.Thread(target=self._monitor, args=(uid, self.cancel, t0), daemon=True).start()

    def stop(self) -> None:
        with self.lock:
            self._stop_locked()

    def close(self) -> None:
        self.stop()
        self.jobs.put((None, None, None, None))
        if self.player is not None:
            self.player.close()

    def _stop_locked(self) -> None:
        if self.current is not None:
            self.cancel.set()
            self.player.stop()
            emit(event="stopped", id=self.current)
            self.current = None

    def _generate(self, uid: int, text: str, cancel: threading.Event, t0: float) -> None:
        try:
            for piece in stream_text(self.engine, text, cancel, t0=t0):
                if cancel.is_set():
                    return
                self.player.enqueue(uid, piece.audio)
            self.player.end_of_input(uid)
        except Exception as e:  # report and keep the server alive
            with self.lock:
                if self.current == uid:
                    cancel.set()
                    self.player.stop()
                    self.current = None
                    emit(event="error", id=uid, message=f"{type(e).__name__}: {e}")

    def _monitor(self, uid: int, cancel: threading.Event, t0: float) -> None:
        """Reports real playback milestones for one utterance (no MLX here)."""
        reported_first = False
        while not cancel.is_set():
            p = self.player
            if p.utterance != uid:
                return
            if not reported_first and p.started_at is not None:
                emit(event="first_audio", id=uid, ms=round((p.started_at - t0) * 1000))
                reported_first = True
            if p.drained():
                with self.lock:
                    if self.current == uid and not cancel.is_set():
                        emit(event="finished", id=uid, underruns=p.underruns, underrun_ms=round(p.underrun_s * 1000), audio_ms=round(p.played_s * 1000))
                        self.current = None
                return
            time.sleep(0.005)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", default="qwen3-clone")
    ap.add_argument("--model", default="mlx-community/Qwen3-TTS-12Hz-1.7B-Base-8bit")
    ap.add_argument("--voice", required=True, help="reference wav (transcript in .txt next to it)")
    ap.add_argument("--gain", type=float, default=1.0)
    ap.add_argument("--no-device", action="store_true", help="simulated output (tests)")
    args = ap.parse_args()

    t0 = time.perf_counter()

    def engine_factory():
        from jarvis_tts import engines

        if args.engine != "qwen3-clone":
            raise ValueError(f"motor no soportado: {args.engine}")
        voice = Path(args.voice)
        return engines.Qwen3Clone(args.model, voice, voice.with_suffix(".txt").read_text())

    server = Server(engine_factory, lambda sr: Player(sr, device_output=not args.no_device, gain=args.gain))
    server.ready.wait()
    if server.startup_error:
        emit(event="fatal", message=server.startup_error)
        sys.exit(1)
    emit(event="ready", engine=args.engine, sample_rate=server.engine.sample_rate, load_ms=round((time.perf_counter() - t0) * 1000), pid=os.getpid())

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            cmd = json.loads(line)
        except json.JSONDecodeError:
            emit(event="error", id=None, message="orden no válida")
            continue
        match cmd.get("cmd"):
            case "speak":
                server.speak(int(cmd["id"]), str(cmd["text"]))
            case "stop":
                server.stop()
            case "shutdown":
                break
            case other:
                emit(event="error", id=None, message=f"orden desconocida: {other}")
    server.close()


if __name__ == "__main__":
    main()
