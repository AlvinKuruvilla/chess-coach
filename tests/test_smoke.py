"""Smoke tests: the package imports and the server registers what it should."""

from __future__ import annotations

import chess_coach
from chess_coach.main import server


def test_version_is_set() -> None:
    assert chess_coach.__version__


async def test_both_tools_are_registered_with_a_derived_schema() -> None:
    tools = {tool.name: tool for tool in await server.list_tools()}
    assert set(tools) == {"review_game", "analyse_position"}

    # The Context parameter is injected by the SDK and must not appear in the
    # schema the client sees; only `game_id` is the caller's to supply.
    review = tools["review_game"].input_schema
    assert set(review["properties"]) == {"game_id", "depth", "threshold_cp"}
    assert review["required"] == ["game_id"]
