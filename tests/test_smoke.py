"""Smoke test: the package imports."""

from __future__ import annotations

import chess_coach


def test_version_is_set() -> None:
    assert chess_coach.__version__
