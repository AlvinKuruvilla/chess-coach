# Chess Coach

Reviews my chess.com games and explains why each mistake was a mistake.

A FastAPI backend (`src/chess_coach`) imports games and runs Stockfish reviews;
a Next.js app (`web/`) shows them on a board.

## Running

Needs `stockfish` on PATH (`brew install stockfish`).

```bash
uv run chess_coach                   # backend on 127.0.0.1:8000
cd web && npm install && npm run dev  # app on localhost:3000; /api proxies to the backend
```

The board is [chessground](https://github.com/lichess-org/chessground), which is
GPL-3.0; distributing a build of `web/` puts it under the GPL.

## Development

This project uses [uv](https://docs.astral.sh/uv/) for dependencies and
tooling.

```bash
uv sync                    # create .venv and install deps + dev tools
uv run pytest              # run tests
uv run ruff check .        # lint
uv run ruff format .       # format
uv run mypy src            # type-check (strict)
uv run pip-audit           # scan dependencies for CVEs
```

Install the git hooks once:

```bash
uv run pre-commit install
```

