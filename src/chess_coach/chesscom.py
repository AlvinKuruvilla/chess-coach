"""Reading a player's finished games from chess.com's public API.

The API is unauthenticated and organised by month: `games/archives` lists one
URL per month the player has games in, and each of those returns
`{"games": [...]}` with the full PGN of every game that ended that month.
Verified against the live API on 2026-11-01.

A month's archive is immutable once the month is over -- games are filed by
the month they *ended*, so nothing can be added to a past month. Only the
newest archive can still change, which is what lets the importer fetch every
other month exactly once.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import httpx

API = "https://api.chess.com/pub"

# chess.com asks API clients to identify themselves in the User-Agent and
# throttles anonymous defaults more aggressively. Contact details are left out
# on purpose: they would go to a third party on every request.
USER_AGENT = "chess-coach/0.1 (personal game review)"


@dataclass(frozen=True)
class ChessComGame:
    """One finished game, reduced to what review and listing need."""

    url: str
    """`https://www.chess.com/game/<class>/<number>`. Stable and unique."""

    end_time: int
    """Unix seconds."""

    time_class: str
    rated: bool
    rules: str
    """`chess` for standard games; variants (chess960, bughouse, ...) differ."""

    white: str
    black: str
    white_rating: int
    black_rating: int
    white_result: str
    black_result: str
    """chess.com's per-side outcome: `win`, `checkmated`, `resigned`, ..."""

    pgn: str
    eco_url: str | None
    accuracies: dict[str, float] | None
    """Present only on games someone has run chess.com Game Review on."""

    @property
    def number(self) -> int:
        """The trailing integer of the URL, which is what the backend exposes
        as the game id."""
        return int(self.url.rsplit("/", 1)[1])


def _parse(raw: dict[str, Any]) -> ChessComGame:
    return ChessComGame(
        url=raw["url"],
        end_time=raw["end_time"],
        time_class=raw["time_class"],
        rated=raw["rated"],
        rules=raw["rules"],
        white=raw["white"]["username"],
        black=raw["black"]["username"],
        white_rating=raw["white"]["rating"],
        black_rating=raw["black"]["rating"],
        white_result=raw["white"]["result"],
        black_result=raw["black"]["result"],
        pgn=raw["pgn"],
        eco_url=raw.get("eco"),
        accuracies=raw.get("accuracies"),
    )


def client() -> httpx.AsyncClient:
    return httpx.AsyncClient(
        headers={"User-Agent": USER_AGENT}, timeout=30.0, follow_redirects=True
    )


async def archive_urls(http: httpx.AsyncClient, username: str) -> list[str]:
    """Monthly archive URLs for `username`, oldest first.

    The API returns them in chronological order; the importer relies on the
    last one being the only month that can still change.
    """
    response = await http.get(f"{API}/player/{username.lower()}/games/archives")
    if response.status_code == 404:
        raise LookupError(f"chess.com has no player named {username!r}")
    response.raise_for_status()
    urls: list[str] = response.json()["archives"]
    return urls


async def archive_games(http: httpx.AsyncClient, url: str) -> list[ChessComGame]:
    """Every game in one monthly archive.

    An entry without a `pgn` is skipped, since there is nothing to review. All
    11 games in the account had one on 2026-11-01; whether chess.com ever
    omits it is unverified, so this is a guard, not a known case.
    """
    response = await http.get(url)
    response.raise_for_status()
    return [_parse(raw) for raw in response.json()["games"] if "pgn" in raw]
