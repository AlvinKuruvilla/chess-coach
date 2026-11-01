"""Finding the mistakes in a game and saying what each one cost.

A game is reviewed by evaluating every position in it once. That single pass
yields all three things a mistake report needs: the evaluation swing that
identifies a mistake, the move that should have been played instead, and --
from the evaluation of the position the mistake led to -- the refutation and
what it takes off the board.

The consequence is derived, never asserted. `Consequence` holds a material
diff computed by playing Stockfish's principal variation forward, so a caller
narrating it cannot invent a tactic that is not there. Measured over 150
sampled mistakes in the 400-999 band, 83% carry a consequence derivable this
way; the rest are positional and come back empty. See
`docs/experiment-metric.md`.
"""

from __future__ import annotations

import io
import shutil
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field

import chess
import chess.engine
import chess.pgn

# Piece values for the material diff. Ordinary centipawn-scale values; the
# king is absent because it never leaves the board, and a mate is reported
# separately rather than as material.
PIECE_VALUE = {
    chess.PAWN: 1,
    chess.KNIGHT: 3,
    chess.BISHOP: 3,
    chess.ROOK: 5,
    chess.QUEEN: 9,
}

# Evaluations are clamped to this before differencing. Without it a swing from
# +9 to +3 registers as a three-pawn mistake, when in truth the position was
# winning before and after and nothing a coach would explain has happened. 15%
# of >=200cp swings in the corpus are of this kind (docs/experiment-metric.md).
EVAL_CAP_CP = 1000

# What a mate is worth when a mate score has to become a number. Far outside
# the cap, so any position with a forced mate clamps to the extreme.
MATE_CP = 10_000


@dataclass(frozen=True)
class Consequence:
    """What the refutation takes, derived from the principal variation."""

    material_swing: int
    """Pawns the mistaking side ends up down over the course of the PV."""

    squares: list[str] = field(default_factory=list)
    """Squares where material changes hands, in the order the PV takes them."""

    mate_in: int | None = None
    """Plies to mate against the mistaking side, when the PV forces one."""

    kind: str = "material"
    """Which of four shapes this consequence has.

    - `material` -- the refutation nets material. The common case, 81% of
      sampled mistakes in the 400-999 band.
    - `mate` -- the refutation forces mate. 2%.
    - `trade_neutral` -- material changes hands and washes out. The mistake is
      real but its cost is not the exchange, so a caller must not narrate the
      captures as the reason. 6%.
    - `positional` -- the refutation moves no material at all. 11%. Nothing
      here explains the mistake; say that rather than reaching for the
      evaluation and inventing a mechanism.

    Shares measured over 150 sampled mistakes; see docs/experiment-metric.md.
    """


@dataclass(frozen=True)
class Mistake:
    ply: int
    move_number: int
    side: str
    played: str
    best: str
    cp_loss: int
    fen_before: str
    refutation: list[str]
    consequence: Consequence


def _material(board: chess.Board, side: chess.Color) -> int:
    return sum(
        PIECE_VALUE[piece_type] * len(board.pieces(piece_type, side))
        for piece_type in PIECE_VALUE
    )


def _cp(score: chess.engine.PovScore) -> int:
    """One evaluation as centipawns from White's point of view."""
    white = score.white()
    mate = white.mate()
    if mate is not None:
        return MATE_CP if mate > 0 else -MATE_CP
    value = white.score()
    return 0 if value is None else value


def _terminal_cp(board: chess.Board) -> int | None:
    """The evaluation of a finished position, or None if the game continues.

    Terminal positions are never handed to the engine. A checkmate scores as
    `Mate(0)` -- mated, with no ply count -- and zero carries no sign, so the
    usual mate handling cannot tell who won and reads a delivered mate as a
    catastrophic blunder by the player who delivered it. Deciding it from the
    board avoids the question, and saves a search that would tell us nothing.
    """
    outcome = board.outcome()
    if outcome is None:
        return None
    if outcome.winner is None:
        return 0
    return MATE_CP if outcome.winner == chess.WHITE else -MATE_CP


def _clamp(centipawns: int) -> int:
    return max(-EVAL_CAP_CP, min(EVAL_CAP_CP, centipawns))


def consequence_of(after: chess.Board, pv: list[chess.Move]) -> Consequence:
    """Play the refutation forward and diff the board.

    `after` is the position the mistake produced, so the side to move is the
    opponent -- the one about to collect. Everything here is a fact about
    squares and pieces; no evaluation enters.
    """
    mistaker = not after.turn
    before_material = _material(after, mistaker) - _material(after, not mistaker)

    walk = after.copy()
    squares: list[str] = []
    for plies, move in enumerate(pv, start=1):
        if walk.is_capture(move):
            squares.append(chess.square_name(move.to_square))
        walk.push(move)
        if walk.is_checkmate() and walk.turn == mistaker:
            after_material = _material(walk, mistaker) - _material(walk, not mistaker)
            return Consequence(
                material_swing=before_material - after_material,
                squares=squares,
                mate_in=plies,
                kind="mate",
            )

    after_material = _material(walk, mistaker) - _material(walk, not mistaker)
    swing = before_material - after_material
    if swing >= 1:
        kind = "material"
    elif squares:
        kind = "trade_neutral"
    else:
        kind = "positional"
    return Consequence(material_swing=swing, squares=squares, kind=kind)


async def review_game(
    pgn_text: str,
    *,
    engine_path: str,
    depth: int,
    threshold_cp: int,
    on_progress: Callable[[int, int], Awaitable[None]] | None = None,
) -> tuple[list[Mistake], dict[str, str]]:
    """Evaluate every position in a game and report the mistakes.

    Returns the mistakes and the game's PGN headers.

    One engine process serves the whole review, but every position is analysed
    under a fresh `game` object, which makes python-chess send `ucinewgame`
    and clear the hash table. That preserves the bit-exactness measured on
    2026-08-18, which assumed `Threads 1` and an empty hash; reusing a warm
    table across positions is enough to change node counts, and a review that
    is not reproducible cannot be compared against another run of itself.
    """
    game = chess.pgn.read_game(io.StringIO(pgn_text))
    if game is None:
        raise ValueError("no game found in the supplied PGN")
    moves = list(game.mainline_moves())
    if not moves:
        raise ValueError("the supplied PGN contains a game with no moves")

    _transport, engine = await chess.engine.popen_uci(engine_path)
    try:
        await engine.configure({"Threads": 1})

        # One pass down the game. `evaluations[i]` and `lines[i]` describe the
        # position reached after i plies, so index 0 is the starting position.
        board = game.board()
        evaluations: list[int] = []
        best_moves: list[chess.Move | None] = []
        principal_variations: list[list[chess.Move]] = []
        positions: list[chess.Board] = []

        total = len(moves) + 1
        for index in range(total):
            positions.append(board.copy())
            terminal = _terminal_cp(board)
            if terminal is not None:
                evaluations.append(terminal)
                principal_variations.append([])
                best_moves.append(None)
            else:
                info = await engine.analyse(
                    board,
                    chess.engine.Limit(depth=depth),
                    game=object(),
                )
                evaluations.append(_cp(info["score"]))
                pv = list(info.get("pv") or [])
                principal_variations.append(pv)
                best_moves.append(pv[0] if pv else None)
            if on_progress is not None:
                await on_progress(index + 1, total)
            if index < len(moves):
                board.push(moves[index])

        mistakes: list[Mistake] = []
        for ply, played in enumerate(moves):
            before, after = evaluations[ply], evaluations[ply + 1]
            mover = positions[ply].turn
            # Centipawn loss is always stated from the mover's point of view,
            # so a White blunder and a Black blunder are the same positive
            # number rather than opposite signs.
            loss = (
                _clamp(before) - _clamp(after)
                if mover == chess.WHITE
                else _clamp(after) - _clamp(before)
            )
            if loss < threshold_cp:
                continue

            best = best_moves[ply]
            refutation = principal_variations[ply + 1]
            mistakes.append(
                Mistake(
                    ply=ply,
                    move_number=positions[ply].fullmove_number,
                    side="white" if mover == chess.WHITE else "black",
                    played=positions[ply].san(played),
                    best=positions[ply].san(best) if best else "",
                    cp_loss=loss,
                    fen_before=positions[ply].fen(),
                    refutation=_variation_san(positions[ply + 1], refutation),
                    consequence=consequence_of(positions[ply + 1], refutation),
                )
            )

        return mistakes, dict(game.headers)
    finally:
        await engine.quit()


def find_stockfish() -> str:
    """Path to the Stockfish binary, or an error that says how to get one.

    Called at startup by both the backend and the MCP server so a missing
    binary fails there with a sentence, not inside the first review as an
    opaque `FileNotFoundError` from a subprocess launch.
    """
    found = shutil.which("stockfish")
    if found is None:
        raise RuntimeError(
            "stockfish was not found on PATH; install it (brew install stockfish) "
            "or put the binary somewhere on PATH"
        )
    return found


def _variation_san(board: chess.Board, pv: list[chess.Move]) -> list[str]:
    walk = board.copy()
    out: list[str] = []
    for move in pv:
        out.append(walk.san(move))
        walk.push(move)
    return out


async def analyse_position(
    fen: str, *, engine_path: str, depth: int
) -> dict[str, object]:
    """Evaluate one position. The unit `review_game` is built out of."""
    board = chess.Board(fen)
    _transport, engine = await chess.engine.popen_uci(engine_path)
    try:
        await engine.configure({"Threads": 1})
        info = await engine.analyse(
            board, chess.engine.Limit(depth=depth), game=object()
        )
        pv = list(info.get("pv") or [])
        return {
            "fen": board.fen(),
            "turn": "white" if board.turn == chess.WHITE else "black",
            "score_cp": _cp(info["score"]),
            "mate_in": info["score"].white().mate(),
            "best_move": board.san(pv[0]) if pv else None,
            "principal_variation": _variation_san(board, pv),
            "depth": info.get("depth"),
            "nodes": info.get("nodes"),
        }
    finally:
        await engine.quit()
