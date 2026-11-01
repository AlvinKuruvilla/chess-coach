"""The MCP server: two tools over stdio.

`review_game` is the product -- a game in, the mistakes and what each one
cost out. `analyse_position` exposes the single-position evaluation it is
built from, which is the useful unit when following up on one moment.

Stockfish is located once at import so a missing binary fails at startup with
something actionable, rather than inside the first tool call as an opaque
`FileNotFoundError` from a subprocess launch.
"""

from __future__ import annotations

import shutil
import sys
from typing import Any

from mcp.server.mcpserver import Context, MCPServer

from chess_coach.analysis import analyse_position as _analyse_position
from chess_coach.analysis import review_game as _review_game

# Default search depth. 16 keeps a 60-ply review near a minute on this machine
# while staying far past the horizon of the mistakes this is meant to find; the
# reproducibility check on 2026-08-18 was run at 20, and nothing here depends on
# that particular number.
DEFAULT_DEPTH = 16

# A move is a mistake when it throws away this much. 200cp is the threshold the
# experiment design settles on: it yields a median of 5 mistakes per game in the
# 400-999 band, where 100cp yields 11 and buries the real errors in noise. See
# docs/experiment-metric.md.
DEFAULT_THRESHOLD_CP = 200

server: MCPServer[Any] = MCPServer(name="chess-coach", version="0.1.0")


def _engine_path() -> str:
    found = shutil.which("stockfish")
    if found is None:
        raise RuntimeError(
            "stockfish was not found on PATH; install it (brew install stockfish) "
            "or put the binary somewhere on PATH"
        )
    return found


@server.tool(
    description=(
        "Review a chess game supplied as PGN. Returns every move that lost at "
        "least `threshold_cp` centipawns, with the move that should have been "
        "played, the refutation, and what that refutation takes off the board. "
        "Every field is derived from the engine and the board -- state only "
        "what is present in the result."
    )
)
async def review_game(
    pgn: str,
    ctx: Context[Any, Any],
    depth: int = DEFAULT_DEPTH,
    threshold_cp: int = DEFAULT_THRESHOLD_CP,
) -> dict[str, Any]:
    async def progress(done: int, total: int) -> None:
        await ctx.report_progress(done, total, f"analysed {done}/{total} positions")

    mistakes, headers = await _review_game(
        pgn,
        engine_path=_engine_path(),
        depth=depth,
        threshold_cp=threshold_cp,
        on_progress=progress,
    )
    return {
        "white": headers.get("White", "?"),
        "black": headers.get("Black", "?"),
        "result": headers.get("Result", "*"),
        "opening": headers.get("Opening"),
        "depth": depth,
        "threshold_cp": threshold_cp,
        "mistake_count": len(mistakes),
        "mistakes": [
            {
                "move_number": m.move_number,
                "side": m.side,
                "played": m.played,
                "best": m.best,
                "cp_loss": m.cp_loss,
                "fen_before": m.fen_before,
                "refutation": m.refutation,
                "cost": {
                    "material_swing": m.consequence.material_swing,
                    "squares": m.consequence.squares,
                    "mate_in": m.consequence.mate_in,
                    # One of material / mate / trade_neutral / positional.
                    # The last two mean the captures listed above are not the
                    # reason the move was bad; narrate the move as a mistake
                    # whose cost this result does not explain, rather than
                    # reaching for the captures to fill the gap.
                    "kind": m.consequence.kind,
                },
            }
            for m in mistakes
        ],
    }


@server.tool(
    description=(
        "Evaluate a single position given as FEN. Returns the score in "
        "centipawns from White's point of view, the best move, and the "
        "principal variation."
    )
)
async def analyse_position(fen: str, depth: int = DEFAULT_DEPTH) -> dict[str, Any]:
    return await _analyse_position(fen, engine_path=_engine_path(), depth=depth)


def main(argv: list[str] | None = None) -> int:
    """Serve over stdio until the client closes stdin."""
    try:
        _engine_path()
    except RuntimeError as err:
        # stdout carries the MCP wire and nothing else; diagnostics go to
        # stderr, which the stdio transport explicitly permits.
        print(f"chess-coach: {err}", file=sys.stderr)
        return 1
    server.run("stdio")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
