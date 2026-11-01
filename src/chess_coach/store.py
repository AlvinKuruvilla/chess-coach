"""SQLite storage for imported games, fetched archives, and finished reviews.

Three tables:

- `archives` -- which chess.com monthly archives have been fetched. A past
  month never changes, so its row is what stops it being fetched again.
- `games` -- one row per game, keyed by chess.com's game URL.
- `reviews` -- one row per (game, depth, threshold). A review is a pure
  function of the PGN and those two parameters, because Stockfish at fixed
  depth with `Threads 1` and an empty hash is bit-exact (checked 2026-08-18,
  see SESSION.md). So a stored review is never stale and is never recomputed.

The connection is used only from the event loop's thread: every route is
`async def`, and FastAPI runs those on the loop, never in its threadpool.
`check_same_thread=False` is still needed because the connection is opened
before the loop starts serving. Queries are small and local, so running them
on the loop without awaiting is cheaper than handing each to a thread.
"""

from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from typing import Any

from chess_coach.chesscom import ChessComGame

SCHEMA = """
CREATE TABLE IF NOT EXISTS archives (
    url        TEXT PRIMARY KEY,
    username   TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS games (
    url          TEXT PRIMARY KEY,
    number       INTEGER NOT NULL UNIQUE,
    username     TEXT NOT NULL,
    end_time     INTEGER NOT NULL,
    time_class   TEXT NOT NULL,
    rated        INTEGER NOT NULL,
    rules        TEXT NOT NULL,
    white        TEXT NOT NULL,
    black        TEXT NOT NULL,
    white_rating INTEGER NOT NULL,
    black_rating INTEGER NOT NULL,
    white_result TEXT NOT NULL,
    black_result TEXT NOT NULL,
    pgn          TEXT NOT NULL,
    eco_url      TEXT,
    accuracies   TEXT
);

CREATE INDEX IF NOT EXISTS games_by_end_time ON games (end_time DESC);

CREATE TABLE IF NOT EXISTS reviews (
    game_url     TEXT NOT NULL REFERENCES games (url),
    depth        INTEGER NOT NULL,
    threshold_cp INTEGER NOT NULL,
    result       TEXT NOT NULL,
    PRIMARY KEY (game_url, depth, threshold_cp)
);
"""


class Store:
    def __init__(self, path: Path | str) -> None:
        if str(path) != ":memory:":
            Path(path).parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(path, check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA foreign_keys = ON")
        self.db.executescript(SCHEMA)

    def close(self) -> None:
        self.db.close()

    # Archives

    def fetched_archives(self) -> set[str]:
        return {row["url"] for row in self.db.execute("SELECT url FROM archives")}

    def mark_fetched(self, url: str, username: str, now: int) -> None:
        self.db.execute(
            "INSERT INTO archives (url, username, fetched_at) VALUES (?, ?, ?) "
            "ON CONFLICT (url) DO UPDATE SET fetched_at = excluded.fetched_at",
            (url, username, now),
        )
        self.db.commit()

    # Games

    def upsert_games(self, username: str, games: list[ChessComGame]) -> int:
        """Insert games not already stored. Returns how many were new.

        Existing rows are left alone rather than overwritten: a game's PGN is
        final once it has ended, and the only field that changes later is
        `accuracies`, which is updated separately below so that running Game
        Review on chess.com after an import still lands here.
        """
        before = self.db.total_changes
        for game in games:
            self.db.execute(
                "INSERT INTO games VALUES "
                "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) "
                "ON CONFLICT (url) DO NOTHING",
                (
                    game.url,
                    game.number,
                    username,
                    game.end_time,
                    game.time_class,
                    int(game.rated),
                    game.rules,
                    game.white,
                    game.black,
                    game.white_rating,
                    game.black_rating,
                    game.white_result,
                    game.black_result,
                    game.pgn,
                    game.eco_url,
                    json.dumps(game.accuracies) if game.accuracies else None,
                ),
            )
        inserted = self.db.total_changes - before
        for game in games:
            if game.accuracies:
                self.db.execute(
                    "UPDATE games SET accuracies = ? "
                    "WHERE url = ? AND accuracies IS NULL",
                    (json.dumps(game.accuracies), game.url),
                )
        self.db.commit()
        return inserted

    def list_games(self, limit: int, offset: int) -> list[sqlite3.Row]:
        return list(
            self.db.execute(
                "SELECT * FROM games ORDER BY end_time DESC LIMIT ? OFFSET ?",
                (limit, offset),
            )
        )

    def count_games(self) -> int:
        row = self.db.execute("SELECT count(*) AS n FROM games").fetchone()
        return int(row["n"])

    def game(self, number: int) -> sqlite3.Row | None:
        row: sqlite3.Row | None = self.db.execute(
            "SELECT * FROM games WHERE number = ?", (number,)
        ).fetchone()
        return row

    # Reviews

    def review(self, url: str, depth: int, threshold_cp: int) -> Any | None:
        row = self.db.execute(
            "SELECT result FROM reviews "
            "WHERE game_url = ? AND depth = ? AND threshold_cp = ?",
            (url, depth, threshold_cp),
        ).fetchone()
        return json.loads(row["result"]) if row else None

    def save_review(self, url: str, depth: int, threshold_cp: int, result: Any) -> None:
        self.db.execute(
            "INSERT OR REPLACE INTO reviews VALUES (?, ?, ?, ?)",
            (url, depth, threshold_cp, json.dumps(result)),
        )
        self.db.commit()
