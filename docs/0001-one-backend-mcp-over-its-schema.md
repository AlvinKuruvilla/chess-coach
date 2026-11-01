# 0001: One backend, MCP over its schema

Status: superseded by [0002](0002-tauri-desktop-app-rust-backend.md)
Date: 2026-11-01
Relates to: [PROBLEM.md](../PROBLEM.md), [async-serve-loop.md](async-serve-loop.md)

Chess-coach is a backend over `analysis.py`, with a web UI and an MCP server as its two
clients. The MCP server reaches the backend through three generic tools over the
backend's OpenAPI schema, and keeps dedicated surfaces only where the protocol has one.

## Context

The MCP server is the whole application today: two tools, `review_game` and
`analyse_position`, calling `analysis.py` directly. A full application adds games,
imports, stored reviews and a UI, and every one of those is something an agent will want
to ask about.

Writing an MCP tool per backend route keeps two descriptions of the same API in step by
hand. The pattern large APIs have moved to is code mode: the agent searches the API
schema and writes code against it, so the tool count does not grow with the route count.
Cloudflare's server covers 2,500 endpoints in about 1k tokens this way.

## Decision

```text
                 analysis.py   (Stockfish + python-chess; no I/O framework)
                      |
                 backend       games, imports, reviews, positions  -> openapi.json
                 /        \
           web UI          MCP server
                           |- search / execute / whoami   over openapi.json
                           |- review_game                  progress, cancellation, Tasks
                           |- resources, prompts,          the surfaces in PROBLEM.md
                              completion, elicitation, ui://
```

- **The backend is FastAPI.** It derives OpenAPI from type hints with no extra
  dependency, and it is async, which the Stockfish subprocess needs
  ([async-serve-loop.md](async-serve-loop.md), section 4).
- **`analysis.py` stays framework-free.** The backend and the MCP server both import it;
  neither owns it.
- **`search`** runs model-written code over `openapi.json` and returns the operations it
  selects. **`execute`** runs model-written code in a sandbox with no network, whose only
  capability is `request(method, path, params, body)`. **`whoami`** returns the user and,
  once Lichess OAuth exists, the scopes.
- **`review_game` keeps its own tool.** `execute` is one request and one capped reply. A
  61-ply review at depth 16 takes 13.6s and emits 62 progress notifications, which is the
  case progress, cancellation and the Tasks extension exist for.
- **Every surface in PROBLEM.md's table stays dedicated.** Resources, prompts,
  completion, pagination, caching, elicitation, subscriptions and the Apps board are not
  tools, so code mode replaces none of them. Goal 1's coverage is unchanged.

## Alternatives

- **One MCP tool per backend route.** Two hand-maintained descriptions of one API, and a
  tool list that grows with the backend.
- **Generate a tool per route from the schema.** No hand-maintenance, but the agent pays
  for every tool definition on every request.
- **No backend; the MCP server is the application.** What exists now. Rules out the web
  UI, and puts storage and imports behind a protocol that only agents speak.
- **Django instead of FastAPI.** Known from Precogly, with an admin and an ORM included.
  Needs drf-spectacular for the schema and is sync by default, which works against the
  engine subprocess.

## Trade-offs

- **Two ways to start a review.** `review_game` and `execute` calling the review route
  both exist. Both must call the same backend function, or they drift.
- **`execute` runs model-written code.** The app is single-user and local, so the
  sandbox guards the machine, not other tenants. It still needs one.
- **Goal 2 gets a layer to read through.** A `tools/call` to `execute` becomes generated
  code, then an HTTP-shaped request, then a route. That is more to trace, and it is
  traceable.
- **The fork in PROBLEM.md is untouched.** Where explanations come from is still settled
  by the experiment, not by this.

## Open

- Which sandbox: `pydantic-monty` (Python, in-process) or a process-level sandbox.
- How `request()` reaches the backend: in-process through the ASGI app, or over HTTP.
- Whether `analyse_position` stays a tool or becomes a route reached through `execute`.

## References

- Cloudflare MCP server, README at
  [`e8bc56f`](https://github.com/cloudflare/mcp/blob/e8bc56f8638bc8ee6b6a2ccc1541b7fab46187e2/README.md).
- Anthropic, [Code execution with MCP](https://www.anthropic.com/research/code-execution-with-mcp).
