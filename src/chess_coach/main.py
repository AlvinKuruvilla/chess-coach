"""The MCP server: two tools over stdio.

`review_game` is the product -- a stored game in, the mistakes and what each
one cost out. It calls `backend.review_stored_game`, the function behind the
backend's review route, so the MCP tool and the HTTP route cannot drift (the
trade-off docs/0001 names). It reads the same SQLite database the backend
writes; games get there through the backend's import route.

`analyse_position` exposes the single-position evaluation reviews are built
from, which is the useful unit when following up on one moment.
"""

from __future__ import annotations

import os
import sys
from typing import Any

from mcp.server.mcpserver import Context, MCPServer

from chess_coach.analysis import analyse_position as _analyse_position
from chess_coach.analysis import find_stockfish
from chess_coach.backend import (
    DEFAULT_DB,
    DEFAULT_DEPTH,
    DEFAULT_THRESHOLD_CP,
    review_stored_game,
)
from chess_coach.store import Store

server: MCPServer[Any] = MCPServer(name="chess-coach", version="0.1.0")

_store: Store | None = None


def _get_store() -> Store:
    global _store
    if _store is None:
        _store = Store(os.environ.get("CHESS_COACH_DB") or DEFAULT_DB)
    return _store


@server.tool(
    description=(
        "Review an imported chess game by id. Returns every move that lost at "
        "least `threshold_cp` centipawns, with the move that should have been "
        "played, the refutation, and what that refutation takes off the board. "
        "Every field is derived from the engine and the board -- state only "
        "what is present in the result. The `cost.kind` values trade_neutral "
        "and positional mean the captures listed are not the reason the move "
        "was bad; say the result does not explain it rather than filling the gap."
    )
)
async def review_game(
    game_id: int,
    ctx: Context[Any, Any],
    depth: int = DEFAULT_DEPTH,
    threshold_cp: int = DEFAULT_THRESHOLD_CP,
) -> dict[str, Any]:
    async def progress(done: int, total: int) -> None:
        await ctx.report_progress(done, total, f"analysed {done}/{total} positions")

    review = await review_stored_game(
        _get_store(),
        game_id,
        depth=depth,
        threshold_cp=threshold_cp,
        on_progress=progress,
    )
    return review.model_dump()


@server.tool(
    description=(
        "Evaluate a single position given as FEN. Returns the score in "
        "centipawns from White's point of view, the best move, and the "
        "principal variation."
    )
)
async def analyse_position(fen: str, depth: int = DEFAULT_DEPTH) -> dict[str, Any]:
    return await _analyse_position(fen, engine_path=find_stockfish(), depth=depth)


def main(argv: list[str] | None = None) -> int:
    """Serve over stdio until the client closes stdin."""
    try:
        find_stockfish()
    except RuntimeError as err:
        # stdout carries the MCP wire and nothing else; diagnostics go to
        # stderr, which the stdio transport explicitly permits.
        print(f"chess-coach: {err}", file=sys.stderr)
        return 1
    server.run("stdio")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
