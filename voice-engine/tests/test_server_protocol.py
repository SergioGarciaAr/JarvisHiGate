"""Sidecar protocol with a fake engine and simulated output (no model, no sound)."""
import json
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from jarvis_tts import server as srv  # noqa: E402
from jarvis_tts.player import Player  # noqa: E402

SR = 24000


class FakeEngine:
    id = "fake"
    sample_rate = SR

    def __init__(self, chunk_s=0.2, gen_delay=0.0, fail_on=None):
        self.chunk_s, self.gen_delay, self.fail_on = chunk_s, gen_delay, fail_on

    def stream(self, text, cancel):
        if self.fail_on and self.fail_on in text:
            raise RuntimeError("fallo simulado del motor")
        for _ in range(3):
            if cancel.is_set():
                return
            time.sleep(self.gen_delay)
            yield (np.ones(int(SR * self.chunk_s)) * 0.2).astype(np.float32)


def make_server(engine, **player_kw):
    s = srv.Server(lambda: engine, lambda sr: Player(sr, device_output=False, **player_kw))
    assert s.ready.wait(5) and s.startup_error is None
    return s


def run(engine, script, wait=3.0, **player_kw):
    events = []
    srv.emit = lambda **e: events.append(e)  # capture instead of stdout
    s = make_server(engine, **player_kw)
    for step in script:
        step(s)
    deadline = time.time() + wait
    while time.time() < deadline and not any(e["event"] in ("finished", "error") and e.get("id") == script_last_id(events) for e in events):
        time.sleep(0.02)
    return events


def script_last_id(events):
    ids = [e.get("id") for e in events if "id" in e]
    return ids[-1] if ids else None


def test_speak_reports_first_audio_then_finished_without_underruns():
    ev = run(FakeEngine(), [lambda s: s.speak(1, "Hola. Qué tal.")])
    kinds = [e["event"] for e in ev]
    assert kinds[0] == "first_audio" and kinds[-1] == "finished", kinds
    fin = ev[-1]
    assert fin["underruns"] == 0
    assert 550 <= fin["audio_ms"] <= 700  # one fragment ("Hola. Qué tal.") x 3 chunks x 0.2 s


def test_new_speak_interrupts_previous_and_old_never_finishes():
    ev = run(FakeEngine(gen_delay=0.05), [lambda s: s.speak(1, "Uno. " * 30), lambda s: time.sleep(0.2), lambda s: s.speak(2, "Dos.")])
    assert {"event": "stopped", "id": 1} in ev
    assert not any(e["event"] == "finished" and e["id"] == 1 for e in ev)
    assert any(e["event"] == "finished" and e["id"] == 2 for e in ev)


def test_stop_is_immediate_and_silences_output():
    events = []
    srv.emit = lambda **e: events.append(e)
    s = make_server(FakeEngine(chunk_s=2.0))
    p = s.player
    s.speak(5, "Frase larga.")
    time.sleep(0.3)
    t = time.perf_counter()
    s.stop()
    assert time.perf_counter() - t < 0.05
    assert {"event": "stopped", "id": 5} in events
    assert p.drained()
    time.sleep(0.3)
    assert not any(e["event"] == "finished" for e in events)


def test_engine_error_is_reported_and_server_survives():
    ev = run(FakeEngine(fail_on="ROMPE"), [lambda s: s.speak(9, "ROMPE esto.")])
    err = [e for e in ev if e["event"] == "error"]
    assert err and err[0]["id"] == 9 and "fallo simulado" in err[0]["message"]


def test_underruns_are_measured_when_generation_is_slower_than_playback():
    # Buffering disabled to measure raw underruns.
    ev = run(FakeEngine(chunk_s=0.05, gen_delay=0.15), [lambda s: s.speak(3, "Lenta.")], wait=5, prebuffer_ms=0, rebuffer_ms=0)
    fin = [e for e in ev if e["event"] == "finished"][0]
    # 3 chunks of 50 ms each generated 150 ms apart: two gaps of ~100 ms.
    # The simulated device clock (time.sleep) is approximate, hence the wide band.
    assert fin["underruns"] == 2 and 100 <= fin["underrun_ms"] <= 260, fin


class ThreadBoundEngine(FakeEngine):
    """Mimics MLX: fails when used from a thread other than the one that created it."""

    def __init__(self):
        super().__init__(gen_delay=0.05)
        self.owner = None

    def stream(self, text, cancel):
        import threading as th

        if self.owner is None:
            self.owner = th.get_ident()
        if th.get_ident() != self.owner:
            raise RuntimeError("There is no Stream(gpu, 2) in current thread.")
        yield from super().stream(text, cancel)


def test_all_synthesis_runs_on_one_thread_even_with_overlapping_requests():
    ev = run(ThreadBoundEngine(), [lambda s: s.speak(1, "Uno. " * 20), lambda s: time.sleep(0.12), lambda s: s.speak(2, "Dos. " * 5), lambda s: time.sleep(0.05), lambda s: s.speak(3, "Tres.")], wait=5)
    assert not [e for e in ev if e["event"] == "error"], ev
    assert {"event": "stopped", "id": 1} in ev and {"event": "stopped", "id": 2} in ev
    assert any(e["event"] == "finished" and e["id"] == 3 for e in ev)


def test_cli_rejects_unknown_engine_with_fatal_event():
    py = sys.executable
    r = subprocess.run([py, str(Path(srv.__file__)), "--engine", "nope", "--voice", "/nonexistent.wav", "--no-device"], capture_output=True, text=True, timeout=120)
    assert r.returncode == 1
    assert json.loads(r.stdout.strip().splitlines()[-1])["event"] == "fatal"


def test_playback_waits_for_prebuffer_then_plays_continuously():
    p = Player(SR, device_output=False, prebuffer_ms=300)
    p.begin(1)
    p.enqueue(1, np.ones(int(SR * 0.1), dtype=np.float32))
    time.sleep(0.2)
    assert p.started_at is None  # 100 ms queued < 300 ms prebuffer
    p.enqueue(1, np.ones(int(SR * 0.3), dtype=np.float32))
    time.sleep(0.15)
    assert p.started_at is not None


def test_short_utterance_shorter_than_prebuffer_still_plays():
    p = Player(SR, device_output=False, prebuffer_ms=500)
    p.begin(2)
    p.enqueue(2, np.ones(int(SR * 0.1), dtype=np.float32))
    p.end_of_input(2)
    time.sleep(0.15)
    assert p.started_at is not None
