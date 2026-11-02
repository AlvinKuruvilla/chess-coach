# 0002: A Tauri desktop app with a Rust backend

Status: accepted
Date: 2026-11-01
Supersedes: [0001](0001-one-backend-mcp-over-its-schema.md)

Chess Coach is a single desktop app. A Tauri shell hosts a React webview, and the
backend -- chess.com import, SQLite storage, Stockfish review -- is Rust, called
from the webview as Tauri commands. There is no HTTP server, no Python, and no MCP
server.

## Context

0001 proposed a FastAPI backend over `analysis.py` with two clients, a web UI and an
MCP server reaching the backend through `search`/`execute` over its OpenAPI schema.
Its first slice was built on 2026-11-01: the backend, then a Next.js UI over it.

Three things changed the same day.

- **The MCP server was set aside** until it earns a place in the app. The app is the
  product; goals 1-2 in PROBLEM.md (cover the MCP specification, read the SDK) are
  paused, not dropped. The last version of the server is in `05a6de1`.
- **A web app needed two servers running** -- FastAPI and Next -- and Next's proxy
  cut off any review longer than 30 seconds, which the 107-ply game was. The app is
  used by one person at one desk, so a browser buys nothing.
- **Next.js cannot be bundled into Tauri as it was written.** Tauri serves static
  files only, and Next's static export does not support the two things the UI
  relied on: rewrites (the `/api` proxy) and dynamic routes without
  `generateStaticParams` (`/games/[id]`).

## Decision

```text
  app/src  (React, Vite)          the webview: games list, board, review
     |  invoke("review_game", ...) + a Channel for progress
  app/src-tauri/src  (Rust)
     |- lib.rs        Tauri commands; first-run account; app state
     |- chesscom.rs   monthly archives from api.chess.com (reqwest)
     |- store.rs      SQLite (rusqlite): games, archives, settings, reviews
     |- analysis.rs   mistakes and what each refutation costs (shakmaty, pgn-reader)
     '- uci.rs        async Stockfish driver over tokio::process
```

- **Rust, not a Python sidecar.** A sidecar would have kept `analysis.py` and its
  tests, but shipping it means bundling Python (PyInstaller), and the app would keep
  two languages and a localhost server indefinitely. The port was checked against
  the Python implementation on the two games it had reviewed (40 and 107 plies,
  depth 16): all nine mistakes matched field for field.
- **Vite, not Next.** The UI was a client-side app already; Next contributed only
  the routing and proxy that Tauri cannot use.
- **Tauri 2, not 3.** crates.io lists `3.0.0-alpha.5` as newest; 2.x is the stable
  line.
- **A hand-written UCI driver.** The `uci` crate (0.2.3) is synchronous, searches by
  `movetime` only, and returns a best move with no score or PV.
- **Progress over a Tauri channel** replaces HTTP request/response for reviews, which
  removed the proxy-timeout problem rather than raising the limit.

## Alternatives

- **Keep the FastAPI backend as a sidecar.** No port, but Python packaging for any
  distribution, and a second process to start and supervise.
- **Next.js with `output: 'export'`.** Works with `/game?id=` routing and Cache
  Components turned off -- which is Vite with more configuration.
- **Stay a web app.** Two dev servers, a proxy timeout, and no use for the browser.

## Trade-offs

- **The MCP learning goals are on hold.** Nothing here exercises the protocol.
  Bringing an MCP server back means deciding what it would be a client of; 0001's
  answer, an OpenAPI schema, no longer exists.
- **Terminal detection is narrower.** shakmaty knows checkmate, stalemate and
  insufficient material but not fivefold repetition or the 75-move rule, which
  python-chess counted. Those positions are now searched; the engine, given the
  history, scores them as draws.
- **chessground is GPL-3.0** and the repository is MIT. Fine while the app is
  private; distributing a build puts it under the GPL.
- **macOS only, in practice.** Nothing is platform-specific in the code, but the
  Stockfish lookup knows Homebrew's paths, CI runs on macOS, and no other platform
  has been tried.

## Consequences for other documents

- 0001 is superseded.
- `async-serve-loop.md` and `mcp-2026-07-28-reference.md` describe the MCP server and
  its Python serve loop. They stay as reference for when MCP returns; section 4 of
  the serve-loop notes (engine lifetime) is the only part that applied to the
  current backend, and `uci.rs` now settles it: one engine per review, killed on
  drop.
- `experiment-metric.md` is unaffected. The experiment is about where explanations
  come from, which this decision does not touch.
