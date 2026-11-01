# Chess Coach

An MCP server that explains why a chess move was bad, built against MCP revision 2026-07-28.

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

