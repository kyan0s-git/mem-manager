#!/usr/bin/env python3
"""Executable reference for docs/architecture/policy-engine.md.

This module is the tie-breaker between the Rust (Windows) and Swift (macOS)
implementations: it implements §4 (EWMA + hysteresis state machine) and §7
(leak detector) exactly as specified, and generates the JSON test vectors in
spec/test-vectors/. Standard library only.

    python3 spec/reference/policy_ref.py --write spec/test-vectors   # regenerate
    python3 spec/reference/policy_ref.py --check spec/test-vectors   # verify
"""
from __future__ import annotations

import argparse
import json
import math
import random
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# §6 profiles (leak thresholds only; the rest is platform policy)
# ---------------------------------------------------------------------------
PROFILES = {
    "conservative": {"slope": 200.0, "abs": 512.0, "rel": 0.40},
    "balanced": {"slope": 100.0, "abs": 256.0, "rel": 0.25},
    "aggressive": {"slope": 50.0, "abs": 128.0, "rel": 0.15},
}
HEAVY_MULTIPLIER = 2.0
RUNAWAY_MIB_PER_H = 1024.0 * 60.0


# ---------------------------------------------------------------------------
# §7 leak detector
# ---------------------------------------------------------------------------
def _median(xs: list[float]) -> float:
    s = sorted(xs)
    n = len(s)
    mid = n // 2
    return s[mid] if n % 2 else (s[mid - 1] + s[mid]) / 2.0


def theil_sen(ts: list[float], ys: list[float]) -> float:
    slopes = [
        (ys[j] - ys[i]) / (ts[j] - ts[i])
        for i in range(len(ts))
        for j in range(i + 1, len(ts))
        if ts[j] > ts[i]
    ]
    return _median(slopes) if slopes else 0.0


def mann_kendall(ys: list[float]) -> tuple[float, float]:
    n = len(ys)
    s = 0
    for i in range(n):
        for j in range(i + 1, n):
            d = ys[j] - ys[i]
            s += (d > 0) - (d < 0)
    ties: dict[float, int] = {}
    for y in ys:
        ties[y] = ties.get(y, 0) + 1
    tie_term = sum(t * (t - 1) * (2 * t + 5) for t in ties.values() if t > 1)
    var = (n * (n - 1) * (2 * n + 5) - tie_term) / 18.0
    if s > 0:
        z = (s - 1) / math.sqrt(var)
    elif s < 0:
        z = (s + 1) / math.sqrt(var)
    else:
        z = 0.0
    tau = s / (n * (n - 1) / 2.0)
    return tau, z


def decimate(series: list[list[float]], max_points: int = 64) -> list[list[float]]:
    n = len(series)
    if n <= max_points:
        return list(series)
    k = math.ceil(n / max_points)
    idx = list(range(n - 1, -1, -k))
    idx.reverse()
    return [series[i] for i in idx]


def analyze(series: list[list[float]], profile: str = "balanced", heavy: bool = False,
            process_age_ok: bool = True) -> dict:
    p = PROFILES[profile]
    mult = HEAVY_MULTIPLIER if heavy else 1.0
    out = {"eligible": False, "slope": 0.0, "tau": 0.0, "z": 0.0,
           "plateau": False, "leak": False, "runaway": False}

    # Runaway rule (§7.8): raw samples within the last 2 minutes.
    if series:
        t_end = series[-1][0]
        tail = [pt for pt in series if pt[0] >= t_end - 2.0 / 60.0]
        if len(tail) >= 5:
            r = theil_sen([pt[0] for pt in tail], [pt[1] for pt in tail])
            out["runaway"] = r >= RUNAWAY_MIB_PER_H

    n_raw = len(series)
    if n_raw < 40 or series[-1][0] - series[0][0] < 0.5 or not process_age_ok:
        return out
    out["eligible"] = True

    d = decimate(series)
    ts = [pt[0] for pt in d]
    ys = [pt[1] for pt in d]
    slope = theil_sen(ts, ys)
    tau, z = mann_kendall(ys)
    n = len(d)
    m = max(8, math.ceil(n / 4))
    s_tail = theil_sen(ts[-m:], ys[-m:])
    plateau = not (s_tail >= 0.5 * slope)
    growth = ys[-1] - ys[0]
    baseline = ys[0]
    leak = (slope >= p["slope"] * mult
            and growth >= max(p["abs"] * mult, p["rel"] * baseline)
            and tau >= 0.6 and z > 2.33 and not plateau)
    out.update(slope=slope, tau=tau, z=z, plateau=plateau, leak=leak)
    return out


# ---------------------------------------------------------------------------
# §4 EWMA + hysteresis state machine
# ---------------------------------------------------------------------------
STATES = ["Normal", "Elevated", "High", "Critical"]
ENTER = [None, 0.40, 0.65, 0.85]
EXIT = [None, 0.30, 0.55, 0.75]
DWELL_S = [0.0, 30.0, 30.0, 60.0]
TAU_S = 20.0
P_EVENT = 0.9


class StateMachine:
    def __init__(self) -> None:
        self.s: float | None = None
        self.t_ms: int | None = None
        self.state = 0
        self.entered_ms = 0

    def step(self, t_ms: int, p: float, event: bool = False) -> str:
        if self.s is None:
            self.s = p
        else:
            dt = (t_ms - self.t_ms) / 1000.0
            alpha = 1.0 - math.exp(-dt / TAU_S)
            self.s += alpha * (p - self.s)
        if event:
            self.s = max(self.s, P_EVENT)
        self.t_ms = t_ms

        # Upward: immediate, may skip states.
        target = self.state
        for i in range(3, self.state, -1):
            if self.s >= ENTER[i]:
                target = i
                break
        if target > self.state:
            self.state, self.entered_ms = target, t_ms
        # Downward: one step, after dwell.
        elif (self.state > 0 and self.s < EXIT[self.state]
              and (t_ms - self.entered_ms) / 1000.0 >= DWELL_S[self.state]):
            self.state, self.entered_ms = self.state - 1, t_ms
        return STATES[self.state]


def run_states(samples: list[list]) -> list[str]:
    sm = StateMachine()
    return [sm.step(int(s[0]), float(s[1]), bool(s[2]) if len(s) > 2 else False) for s in samples]


# ---------------------------------------------------------------------------
# Vector generation
# ---------------------------------------------------------------------------
def _series(fn, hours: float, step_s: float = 30.0) -> list[list[float]]:
    n = int(hours * 3600 / step_s) + 1
    return [[round(i * step_s / 3600.0, 6), round(fn(i * step_s / 3600.0, i), 3)] for i in range(n)]


def leak_vectors() -> list[dict]:
    rng = random.Random(1234)
    noise = lambda a: rng.uniform(-a, a)  # noqa: E731
    v = []

    def add(name, series, profile="balanced", heavy=False):
        v.append({"name": name, "profile": profile, "heavy": heavy, "series": series,
                  "expect": analyze(series, profile, heavy)})

    add("linear-leak-300mib-h", _series(lambda t, i: 400 + 300 * t + noise(15), 2.0))
    add("slow-drift-40mib-h", _series(lambda t, i: 900 + 40 * t + noise(10), 2.0))
    add("gc-sawtooth-flat", _series(lambda t, i: 800 + (i % 20) * 12 + noise(5), 2.0))
    add("sawtooth-on-leak", _series(lambda t, i: 500 + 250 * t + (i % 20) * 15 + noise(5), 2.0))
    add("cache-fill-plateau", _series(lambda t, i: 300 + 900 * (1 - math.exp(-t * 6)) + noise(4), 2.0))
    add("step-allocation", _series(lambda t, i: 600 + (700 if t > 1.0 else 0) + noise(5), 2.0))
    add("noisy-flat", _series(lambda t, i: 1200 + noise(80), 2.0))
    add("too-short-20min", _series(lambda t, i: 400 + 600 * t, 20 / 60))
    add("heavy-browser-150mib-h", _series(lambda t, i: 2500 + 150 * t + noise(30), 2.0), heavy=True)
    add("heavy-browser-450mib-h", _series(lambda t, i: 2500 + 450 * t + noise(30), 2.0), heavy=True)
    add("aggressive-60mib-h", _series(lambda t, i: 200 + 80 * t + noise(3), 2.0), profile="aggressive")
    add("conservative-150mib-h", _series(lambda t, i: 900 + 150 * t + noise(5), 2.0), profile="conservative")
    runaway = _series(lambda t, i: 500 + 50 * t, 1.0, step_s=30.0)
    t0 = runaway[-1][0]
    for k in range(1, 9):  # 8 samples, 15 s apart, +400 MiB each => 1600 MiB/min
        runaway.append([round(t0 + k * 15 / 3600.0, 6), round(runaway[-1][1] + 400.0, 3)])
    add("runaway-1600mib-min", runaway)
    return v


def state_vectors() -> list[dict]:
    v = []

    def add(name, samples):
        v.append({"name": name, "samples": samples, "expect_states": run_states(samples)})

    add("ramp-up-and-recover",
        [[i * 5000, min(1.0, i * 0.05)] for i in range(21)] + [[100000 + i * 5000, 0.1] for i in range(1, 41)])
    add("flap-inside-hysteresis",
        [[i * 5000, 0.42 if i % 2 else 0.33] for i in range(60)])
    add("emergency-event",
        [[0, 0.1], [5000, 0.1], [10000, 0.1, True], [15000, 0.1], [20000, 0.1]] +
        [[20000 + i * 10000, 0.05] for i in range(1, 30)])
    add("irregular-sampling",
        [[0, 0.2], [1000, 0.9], [1500, 0.9], [61500, 0.9], [62000, 0.2], [182000, 0.2], [302000, 0.0]])
    return v


def write(dirpath: Path) -> None:
    dirpath.mkdir(parents=True, exist_ok=True)
    for vec in leak_vectors():
        (dirpath / f"leak-{vec['name']}.json").write_text(json.dumps(vec, indent=1) + "\n")
    for vec in state_vectors():
        (dirpath / f"state-{vec['name']}.json").write_text(json.dumps(vec, indent=1) + "\n")


def _close(a, b) -> bool:
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    return math.isclose(a, b, rel_tol=1e-9, abs_tol=1e-9)


def check(dirpath: Path) -> int:
    failures = 0
    for f in sorted(dirpath.glob("*.json")):
        vec = json.loads(f.read_text())
        if f.name.startswith("leak-"):
            got = analyze(vec["series"], vec["profile"], vec["heavy"])
            bad = [k for k, e in vec["expect"].items() if not _close(got[k], e)]
        else:
            bad = [] if run_states(vec["samples"]) == vec["expect_states"] else ["states"]
        if bad:
            failures += 1
            print(f"FAIL {f.name}: {bad}")
    print(f"{failures} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--write", type=Path)
    g.add_argument("--check", type=Path)
    a = ap.parse_args()
    if a.write:
        write(a.write)
    else:
        sys.exit(check(a.check))
