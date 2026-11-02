# /// script
# requires-python = ">=3.12"
# dependencies = ["numpy>=2", "scipy>=1.13"]
# ///
"""Fit the evaluation-to-winning-chances curve on Lichess games, by rating band.

The model and filters follow lila PR #11148, which produced Lichess's constant
0.00368208 (https://github.com/lichess-org/lila/pull/11148):

    winning_chances(cp) = 2 / (1 + exp(-k * cp)) - 1        in [-1, 1]

fitted with `scipy.optimize.curve_fit` against the game result from White's
side (1, 0, -1), one point per evaluated position. Abandoned games, time
forfeits and games of four moves or fewer are dropped. Evaluations are capped
at +-1000cp, as scalachess's `cp.ceiled` does; mate scores are dropped.

Two departures from the PR, both deliberate:

- No hand-placed anchor points. The PR added them to keep the curve
  compatible with older Lichess models; here the question is what the data
  says.
- A confidence interval. Positions from one game share one result, so they
  are not independent; the interval comes from resampling whole games.

Usage: uv run scripts/fit_win_curve.py tmp/lichess_2026-07.head.pgn
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from scipy.optimize import curve_fit

LICHESS_K = 0.00368208
CAP_CP = 1000
BOOTSTRAP_ROUNDS = 200
RESULT = {"1-0": 1.0, "0-1": -1.0, "1/2-1/2": 0.0}

HEADER = re.compile(r'^\[(\w+) "(.*)"\]$', re.MULTILINE)
EVAL = re.compile(r"\[%eval (-?\d+(?:\.\d+)?|#-?\d+)\]")
MOVE_NUMBER = re.compile(r"\b(\d+)\.(?!\.)")


@dataclass
class Band:
    name: str
    low: int
    high: int
    include_time_forfeits: bool = False
    # Base time in seconds at or above which a game is kept; 480 is the
    # 8-minute floor in the PR's data preparation.
    min_base_seconds: int = 0
    games: list[tuple[np.ndarray, float]] = field(default_factory=list)


def winning_chances(cp: np.ndarray, k: float) -> np.ndarray:
    return 2 / (1 + np.exp(-k * cp)) - 1


def fit(games: list[tuple[np.ndarray, float]]) -> float:
    x = np.concatenate([evals for evals, _ in games])
    y = np.concatenate([np.full(len(evals), result) for evals, result in games])
    (k,), _ = curve_fit(winning_chances, x, y, p0=[LICHESS_K])
    return float(k)


def main(path: Path) -> None:
    bands = [
        Band("400-999 (your band)", 400, 999),
        Band("400-999, time forfeits kept", 400, 999, include_time_forfeits=True),
        Band("1000-1499", 1000, 1499),
        Band("400-999, rapid and slower", 400, 999, min_base_seconds=480),
        Band("2300+, all time controls", 2300, 4000),
        Band("2300+, rapid and slower (as the PR)", 2300, 4000, min_base_seconds=480),
    ]

    text = path.read_text(encoding="utf-8", errors="replace")
    for chunk in text.split("\n\n[Event ")[1:]:
        headers = dict(HEADER.findall("[Event " + chunk))
        result = RESULT.get(headers.get("Result", ""))
        termination = headers.get("Termination", "")
        if result is None or termination == "Abandoned":
            continue
        try:
            white, black = int(headers["WhiteElo"]), int(headers["BlackElo"])
        except (KeyError, ValueError):
            continue
        evals_raw = EVAL.findall(chunk)
        if not evals_raw:
            continue
        numbers = MOVE_NUMBER.findall(chunk)
        if not numbers or int(numbers[-1]) <= 4:
            continue
        evals = np.array(
            [float(e) * 100 for e in evals_raw if not e.startswith("#")], dtype=float
        )
        if evals.size == 0:
            continue
        evals = np.clip(evals, -CAP_CP, CAP_CP)
        control = headers.get("TimeControl", "-")
        base = int(control.split("+")[0]) if control[0].isdigit() else 10**9
        for band in bands:
            if termination == "Time forfeit" and not band.include_time_forfeits:
                continue
            if base < band.min_base_seconds:
                continue
            if band.low <= white <= band.high and band.low <= black <= band.high:
                band.games.append((evals, result))

    rng = np.random.default_rng(20261101)
    print(f"Lichess constant: {LICHESS_K}\n")
    print(f"{'band':38} {'games':>6} {'positions':>9} {'k':>10}  95% CI (games resampled)")
    for band in bands:
        if len(band.games) < 50:
            print(f"{band.name:38} {len(band.games):>6}  too few games")
            continue
        k = fit(band.games)
        boot = []
        for _ in range(BOOTSTRAP_ROUNDS):
            picks = rng.integers(0, len(band.games), len(band.games))
            boot.append(fit([band.games[i] for i in picks]))
        low, high = np.percentile(boot, [2.5, 97.5])
        positions = sum(len(e) for e, _ in band.games)
        inside = "contains" if low <= LICHESS_K <= high else "excludes"
        print(
            f"{band.name:38} {len(band.games):>6} {positions:>9} {k:>10.6f}  "
            f"[{low:.6f}, {high:.6f}]  {inside} Lichess's k"
        )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: fit_win_curve.py <lichess PGN with %eval annotations>")
    main(Path(sys.argv[1]))
