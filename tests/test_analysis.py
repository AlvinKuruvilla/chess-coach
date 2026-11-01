"""Tests for mistake detection and the consequence diff.

`consequence_of` is pure -- a board and a move list in, a diff out -- so its
four outcomes are tested without an engine. The game-level tests do need
Stockfish, and run at a shallow depth: they check bookkeeping (which side
erred, that the loss is positive) rather than the engine's judgement.
"""

from __future__ import annotations

import shutil

import chess
import pytest

from chess_coach.analysis import consequence_of, review_game


def _consequence(fen: str, pv_san: list[str]):
    board = chess.Board(fen)
    walk = board.copy()
    pv = []
    for san in pv_san:
        move = walk.parse_san(san)
        pv.append(move)
        walk.push(move)
    return consequence_of(board, pv)


def test_material_won_is_counted_from_the_mistaker_side() -> None:
    # Black to move with a knight on c2 and White's rook undefended on a1.
    c = _consequence("4k3/8/8/8/8/8/2n5/R3K3 b - - 0 1", ["Nxa1"])
    assert c.kind == "material"
    assert c.material_swing == 5
    assert c.squares == ["a1"]


def test_back_rank_mate_is_its_own_kind() -> None:
    c = _consequence("r5k1/5ppp/8/8/8/8/5PPP/6K1 b - - 0 1", ["Ra1#"])
    assert c.kind == "mate"
    assert c.mate_in == 1


def test_an_even_trade_is_not_reported_as_winning_material() -> None:
    # Rooks come off on d8. Material changes hands twice and nets to nothing,
    # so narrating the captures as the cost would be a false explanation.
    c = _consequence("3rk3/8/8/8/8/8/8/3RK3 w - - 0 1", ["Rxd8+", "Kxd8"])
    assert c.kind == "trade_neutral"
    assert c.material_swing == 0
    assert c.squares == ["d8", "d8"]


def test_a_quiet_refutation_reports_nothing_rather_than_guessing() -> None:
    c = _consequence(
        "r1bqkbnr/pppppppp/2n5/8/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 1",
        ["Bb5", "a6", "Ba4"],
    )
    assert c.kind == "positional"
    assert c.material_swing == 0
    assert c.squares == []


needs_stockfish = pytest.mark.skipif(
    shutil.which("stockfish") is None, reason="stockfish is not on PATH"
)

# 3... Nf6 walks into Qxf7#. A Black mistake, which is the case where a sign
# error in the centipawn-loss calculation would show up as a negative loss or
# as the blunder being attributed to White.
SCHOLARS_MATE = """[Event "test"]
[White "w"]
[Black "b"]
[Result "1-0"]

1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7# 1-0
"""


@needs_stockfish
async def test_a_black_mistake_is_attributed_to_black_with_a_positive_loss() -> None:
    mistakes, headers = await review_game(
        SCHOLARS_MATE,
        engine_path=str(shutil.which("stockfish")),
        depth=8,
        threshold_cp=200,
    )
    assert headers["Result"] == "1-0"
    blunder = next(m for m in mistakes if m.played == "Nf6")
    assert blunder.side == "black"
    assert blunder.cp_loss > 0
    assert blunder.move_number == 3


@needs_stockfish
async def test_progress_is_reported_once_per_position() -> None:
    seen: list[tuple[int, int]] = []

    async def record(done: int, total: int) -> None:
        seen.append((done, total))

    _mistakes, _headers = await review_game(
        SCHOLARS_MATE,
        engine_path=str(shutil.which("stockfish")),
        depth=6,
        on_progress=record,
        threshold_cp=200,
    )
    # Seven plies, so eight positions including the start.
    assert [done for done, _ in seen] == list(range(1, 9))
    assert {total for _, total in seen} == {8}


@needs_stockfish
async def test_a_pgn_with_no_moves_is_rejected_at_the_boundary() -> None:
    with pytest.raises(ValueError, match="no moves"):
        await review_game(
            '[Event "empty"]\n\n*\n',
            engine_path=str(shutil.which("stockfish")),
            depth=6,
            threshold_cp=200,
        )
