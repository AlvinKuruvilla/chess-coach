"""The backend's import, listing, and review routes.

chess.com is replaced by an `httpx.MockTransport` serving two real games from
the account's 2026-10 archive (tests/fixtures/chesscom-2026-10.json, fetched
2026-11-01): one with chess.com accuracies and one without, because the
accuracy backfill in `Store.upsert_games` depends on that difference.

The archive list names two months. The older one is served once and then
fails if requested again -- that is the only way to observe that a past month
is not refetched.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import httpx
import pytest
from fastapi.testclient import TestClient

from chess_coach import backend, chesscom

FIXTURE = json.loads(
    (Path(__file__).parent / "fixtures" / "chesscom-2026-10.json").read_text()
)
ARCHIVES = f"{chesscom.API}/player/alvinkuruvilla02/games/archives"
SEPTEMBER = f"{chesscom.API}/player/alvinkuruvilla02/games/2026/09"
OCTOBER = f"{chesscom.API}/player/alvinkuruvilla02/games/2026/10"


@pytest.fixture
def requests(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    seen: list[str] = []

    def handle(request: httpx.Request) -> httpx.Response:
        url = str(request.url)
        seen.append(url)
        if url == ARCHIVES:
            return httpx.Response(200, json={"archives": [SEPTEMBER, OCTOBER]})
        if url == SEPTEMBER:
            if seen.count(SEPTEMBER) > 1:
                return httpx.Response(500, text="past month refetched")
            return httpx.Response(200, json={"games": []})
        if url == OCTOBER:
            return httpx.Response(200, json=FIXTURE)
        return httpx.Response(404, json={"code": 0, "message": "not found"})

    def client() -> httpx.AsyncClient:
        return httpx.AsyncClient(transport=httpx.MockTransport(handle))

    monkeypatch.setattr(chesscom, "client", client)
    return seen


@pytest.fixture
def api(tmp_path: Path) -> Any:
    with TestClient(backend.create_app(tmp_path / "db.sqlite3")) as client:
        yield client


def test_import_then_reimport_fetches_only_the_newest_month(
    api: Any, requests: list[str]
) -> None:
    first = api.post("/games/import", params={"username": "alvinkuruvilla02"})
    assert first.json() == {
        "username": "alvinkuruvilla02",
        "archives_fetched": 2,
        "archives_skipped": 0,
        "games_new": 2,
    }

    second = api.post("/games/import", params={"username": "alvinkuruvilla02"})
    assert second.json()["archives_fetched"] == 1
    assert second.json()["archives_skipped"] == 1
    assert second.json()["games_new"] == 0
    assert requests.count(SEPTEMBER) == 1
    assert requests.count(OCTOBER) == 2


def test_unknown_player_is_a_404(api: Any, requests: list[str]) -> None:
    response = api.post("/games/import", params={"username": "nobody-here"})
    assert response.status_code == 404
    assert "nobody-here" in response.json()["detail"]


def test_games_are_listed_newest_first_and_addressed_by_url_number(
    api: Any, requests: list[str]
) -> None:
    api.post("/games/import", params={"username": "alvinkuruvilla02"})
    page = api.get("/games").json()
    assert page["total"] == 2
    ends = [g["end_time"] for g in page["games"]]
    assert ends == sorted(ends, reverse=True)

    game = api.get("/games/1000000001").json()
    assert game["url"] == "https://www.chess.com/game/daily/1000000001"
    assert game["accuracies"] == {"white": 71.67, "black": 89.31}
    # Imported as lowercase; the game record has `AlvinKuruvilla02`.
    assert game["you"] == "white"
    assert game["pgn"].startswith('[Event "Let\'s Play!"]')

    assert api.get("/games/1").status_code == 404


def test_review_is_stored_and_served_again_without_the_engine(
    api: Any, requests: list[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    api.post("/games/import", params={"username": "alvinkuruvilla02"})
    first = api.post("/games/1000000002/review", params={"depth": 6})
    assert first.status_code == 200
    review = first.json()
    assert review["depth"] == 6
    assert review["mistake_count"] == len(review["mistakes"])

    # A second identical request must not start Stockfish: make it impossible.
    async def no_engine(*args: Any, **kwargs: Any) -> Any:
        raise AssertionError("the engine ran for a stored review")

    monkeypatch.setattr(backend, "_review_game", no_engine)
    again = api.post("/games/1000000002/review", params={"depth": 6})
    assert again.json() == review


def test_schema_names_every_route(api: Any) -> None:
    paths = set(api.get("/openapi.json").json()["paths"])
    assert paths == {
        "/games/import",
        "/games",
        "/games/{game_id}",
        "/games/{game_id}/review",
    }


def test_a_review_stored_under_an_older_schema_is_recomputed(
    api: Any, requests: list[str]
) -> None:
    api.post("/games/import", params={"username": "alvinkuruvilla02"})
    store = api.app.state.store
    url = "https://www.chess.com/game/daily/1000000002"
    # The shape stored before `ply` was added to each mistake.
    store.save_review(
        url,
        6,
        200,
        {
            "white": "?",
            "black": "?",
            "result": "*",
            "opening": None,
            "depth": 6,
            "threshold_cp": 200,
            "mistake_count": 1,
            "mistakes": [
                {
                    "move_number": 1,
                    "side": "white",
                    "played": "e4",
                    "best": "d4",
                    "cp_loss": 300,
                    "fen_before": "",
                    "refutation": [],
                    "cost": {
                        "material_swing": 0,
                        "squares": [],
                        "mate_in": None,
                        "kind": "positional",
                    },
                }
            ],
        },
    )
    review = api.post("/games/1000000002/review", params={"depth": 6}).json()
    assert all("ply" in m for m in review["mistakes"])
    assert review["white"] != "?"
