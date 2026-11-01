"""The HTTP backend: imported games and their reviews.

It owns storage and the chess.com import. Anything else that starts a review
should call `review_stored_game`, the function behind the review route, so
reviews have one implementation and one cache.

Games are addressed by the trailing integer of their chess.com URL
(`.../game/daily/1000000001` is game `1000000001`). The URL itself is the
primary key in storage; the integer is what goes in paths, because a URL does
not. NOTE: chess.com numbers daily and live games from separate sequences,
so a collision between the two is possible in principle. The `UNIQUE`
constraint on `games.number` makes that an import error rather than a silent
overwrite. None of the 11 games in the account on 2026-11-01 collide.
"""

from __future__ import annotations

import json
import os
import time
from collections.abc import AsyncIterator, Awaitable, Callable
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Annotated, Any

from fastapi import Depends, FastAPI, HTTPException, Query, Request
from pydantic import BaseModel, ValidationError

from chess_coach import chesscom
from chess_coach.analysis import find_stockfish
from chess_coach.analysis import review_game as _review_game
from chess_coach.store import Store

# Default search depth. 16 keeps a 60-ply review near a minute on this machine
# while staying far past the horizon of the mistakes this is meant to find; the
# reproducibility check on 2026-08-18 was run at 20, and nothing here depends on
# that particular number.
DEFAULT_DEPTH = 16

# A move is a mistake when it throws away this much. See
# docs/experiment-metric.md for why 200cp and not 100 or 300.
DEFAULT_THRESHOLD_CP = 200

# Where the database lives unless CHESS_COACH_DB says otherwise. Under tmp/
# because tmp/ is git-ignored and the database holds nothing that cannot be
# re-imported and re-reviewed.
DEFAULT_DB = Path("tmp/chess-coach.sqlite3")


# Response models. These are the OpenAPI schema; field docs land in it.


class Cost(BaseModel):
    material_swing: int
    squares: list[str]
    mate_in: int | None
    kind: str
    """One of material / mate / trade_neutral / positional. The last two mean
    any captures in the refutation are not the reason the move was bad."""


class MistakeOut(BaseModel):
    ply: int
    """Half-moves played before this one; 0 is White's first move. The board
    position before the mistake is the one after `ply` half-moves."""
    move_number: int
    side: str
    played: str
    best: str
    cp_loss: int
    fen_before: str
    refutation: list[str]
    cost: Cost


class Review(BaseModel):
    white: str
    black: str
    result: str
    opening: str | None
    depth: int
    threshold_cp: int
    mistake_count: int
    mistakes: list[MistakeOut]


class GameSummary(BaseModel):
    id: int
    url: str
    end_time: int
    time_class: str
    rated: bool
    white: str
    black: str
    white_rating: int
    black_rating: int
    white_result: str
    black_result: str
    eco_url: str | None
    has_accuracies: bool
    you: str | None
    """`white` or `black`: the side played by the account the game was imported
    for. `None` only if neither player matches, which should not happen."""


class GameDetail(GameSummary):
    pgn: str
    accuracies: dict[str, float] | None


class GamePage(BaseModel):
    total: int
    offset: int
    games: list[GameSummary]


class ImportResult(BaseModel):
    username: str
    archives_fetched: int
    archives_skipped: int
    games_new: int


# The review function both clients call.


async def review_pgn(
    pgn: str,
    *,
    depth: int,
    threshold_cp: int,
    on_progress: Callable[[int, int], Awaitable[None]] | None = None,
) -> Review:
    mistakes, headers = await _review_game(
        pgn,
        engine_path=find_stockfish(),
        depth=depth,
        threshold_cp=threshold_cp,
        on_progress=on_progress,
    )
    return Review(
        white=headers.get("White", "?"),
        black=headers.get("Black", "?"),
        result=headers.get("Result", "*"),
        opening=headers.get("Opening"),
        depth=depth,
        threshold_cp=threshold_cp,
        mistake_count=len(mistakes),
        mistakes=[
            MistakeOut(
                ply=m.ply,
                move_number=m.move_number,
                side=m.side,
                played=m.played,
                best=m.best,
                cp_loss=m.cp_loss,
                fen_before=m.fen_before,
                refutation=m.refutation,
                cost=Cost(
                    material_swing=m.consequence.material_swing,
                    squares=m.consequence.squares,
                    mate_in=m.consequence.mate_in,
                    kind=m.consequence.kind,
                ),
            )
            for m in mistakes
        ],
    )


async def review_stored_game(
    store: Store,
    game_id: int,
    *,
    depth: int,
    threshold_cp: int,
    on_progress: Callable[[int, int], Awaitable[None]] | None = None,
) -> Review:
    """Review a stored game, from the cache when this depth and threshold have
    been run before. A cache hit emits no progress."""
    row = store.game(game_id)
    if row is None:
        raise LookupError(f"no game with id {game_id}; import games first")
    cached = store.review(row["url"], depth, threshold_cp)
    if cached is not None:
        # A review stored before a field was added to `Review` no longer
        # validates. Recomputing it is cheaper than migrating stored JSON, and
        # the result is identical apart from the new field, since the engine
        # is deterministic.
        try:
            return Review.model_validate(cached)
        except ValidationError:
            pass
    review = await review_pgn(
        row["pgn"], depth=depth, threshold_cp=threshold_cp, on_progress=on_progress
    )
    store.save_review(row["url"], depth, threshold_cp, review.model_dump())
    return review


async def import_games(store: Store, username: str) -> ImportResult:
    """Fetch every monthly archive not yet stored, plus the newest one.

    A past month is fetched once and never again (see chesscom.py for why it
    cannot change). The newest month is always refetched, because games keep
    landing in it until the month ends.
    """
    fetched = store.fetched_archives()
    async with chesscom.client() as http:
        urls = await chesscom.archive_urls(http, username)
        newest = urls[-1] if urls else None
        to_fetch = [u for u in urls if u not in fetched or u == newest]
        new = 0
        for url in to_fetch:
            games = await chesscom.archive_games(http, url)
            new += store.upsert_games(username, games)
            store.mark_fetched(url, username, int(time.time()))
    return ImportResult(
        username=username,
        archives_fetched=len(to_fetch),
        archives_skipped=len(urls) - len(to_fetch),
        games_new=new,
    )


# The app.


def _summary(row: Any) -> GameSummary:
    # chess.com usernames are case-insensitive, and the PGN and the API
    # disagree on case: the import is for `alvinkuruvilla02`, the game record
    # says `AlvinKuruvilla02`.
    you = {
        row["white"].lower(): "white",
        row["black"].lower(): "black",
    }.get(row["username"].lower())
    return GameSummary(
        you=you,
        id=row["number"],
        url=row["url"],
        end_time=row["end_time"],
        time_class=row["time_class"],
        rated=bool(row["rated"]),
        white=row["white"],
        black=row["black"],
        white_rating=row["white_rating"],
        black_rating=row["black_rating"],
        white_result=row["white_result"],
        black_result=row["black_result"],
        eco_url=row["eco_url"],
        has_accuracies=row["accuracies"] is not None,
    )


def get_store(request: Request) -> Store:
    store: Store = request.app.state.store
    return store


# Module level, not inside `create_app`: with postponed annotations FastAPI
# resolves this name from the module's globals, and a local is invisible there.
StoreDep = Annotated[Store, Depends(get_store)]


def create_app(db_path: Path | str | None = None) -> FastAPI:
    path = db_path or os.environ.get("CHESS_COACH_DB") or DEFAULT_DB

    @asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        find_stockfish()
        app.state.store = Store(path)
        try:
            yield
        finally:
            app.state.store.close()

    app = FastAPI(title="chess-coach", version="0.1.0", lifespan=lifespan)

    @app.post("/games/import")
    async def import_route(store: StoreDep, username: str) -> ImportResult:
        """Import a chess.com player's finished games."""
        try:
            return await import_games(store, username)
        except LookupError as err:
            raise HTTPException(404, str(err)) from err

    @app.get("/games")
    async def list_route(
        store: StoreDep,
        limit: Annotated[int, Query(ge=1, le=200)] = 50,
        offset: Annotated[int, Query(ge=0)] = 0,
    ) -> GamePage:
        """Imported games, most recently finished first."""
        return GamePage(
            total=store.count_games(),
            offset=offset,
            games=[_summary(row) for row in store.list_games(limit, offset)],
        )

    @app.get("/games/{game_id}")
    async def game_route(store: StoreDep, game_id: int) -> GameDetail:
        """One game, with its PGN."""
        row = store.game(game_id)
        if row is None:
            raise HTTPException(404, f"no game with id {game_id}; import first")
        return GameDetail(
            **_summary(row).model_dump(),
            pgn=row["pgn"],
            accuracies=json.loads(row["accuracies"]) if row["accuracies"] else None,
        )

    @app.post("/games/{game_id}/review")
    async def review_route(
        store: StoreDep,
        game_id: int,
        depth: Annotated[int, Query(ge=1, le=30)] = DEFAULT_DEPTH,
        threshold_cp: Annotated[int, Query(ge=1)] = DEFAULT_THRESHOLD_CP,
    ) -> Review:
        """Review a game: every move that lost at least `threshold_cp`.

        Takes seconds to minutes on first call; repeat calls with the same
        depth and threshold return the stored result.
        """
        try:
            return await review_stored_game(
                store, game_id, depth=depth, threshold_cp=threshold_cp
            )
        except LookupError as err:
            raise HTTPException(404, str(err)) from err

    return app


app = create_app()


def main() -> None:
    import uvicorn

    uvicorn.run(app, host="127.0.0.1", port=8000)
