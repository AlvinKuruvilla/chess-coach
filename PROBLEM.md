# Problem

A chess engine can tell you a move was bad. It cannot tell you why in a way a
1000-rated player can act on.

Stockfish, given the position after a blunder, reports an evaluation swing:
`+0.3` to `-1.4`. That number is ground truth about the move's quality and
carries no instruction. The player already knew the move was bad — the game
result told them. What they need is the mechanism: which piece became loose,
which square stopped being defended, which plan the move abandoned.

This project is a coach that closes that gap for games I have already played,
built as a full application: a backend, a web UI, and an MCP server. It is also
a vehicle for learning MCP against the current specification.

## Goals

One application, not a standalone MCP server. A backend owns games, imports and
reviews and publishes an OpenAPI schema; the web UI and the MCP server are both
clients of it. Architecture in
[docs/0001](docs/0001-one-backend-mcp-over-its-schema.md). The goals below are
all met by building that, and none is pursued apart from it.

1. **Know the specification.** Every non-deprecated surface in the 2026-07-28
   revision, including the parts a tools-only server never touches, exercised
   by something with a real use rather than a stub written to tick a box.
2. **Be able to reason through the code.** Read a wire trace and say what the
   SDK will do with it; open the Python SDK and follow how a `tools/call`
   becomes a function invocation; know where the framework's convenience ends
   and the protocol begins. Knowing the spec and being able to follow an
   implementation of it are different skills, and a server built entirely out
   of decorators teaches only the first.
3. **Get better at chess.** The measure of whether the coaching works. Not
   "produce reviews" — produce reviews that change how I play. If my game
   quality is unchanged in three months, the coaching failed however good the
   application is.

Goal 2 has an immediate consequence, unresolved: see the SDK-layer question in
SESSION.md. FastMCP advertises that you write no JSON Schema, no request
parsing, no validation, and no protocol handling — which is exactly the
material goal 2 is about.

## Why this domain and not another

The learning goal could ride on any server. This domain was chosen for three
properties that are jointly rare:

- **A free oracle.** Stockfish is ground truth for move quality, runs locally,
  and is deterministic at fixed depth. Most hobby projects have no oracle at
  all, so "does it work" degrades into "does it look right".
- **A free real corpus of my own data.** `api.chess.com/pub/player/{user}/games/{YYYY}/{MM}`
  returns `{"games": [...]}` unauthenticated, each entry carrying a full `pgn`,
  the ECO code, the final FEN, and chess.com's own accuracy score for both
  players. Verified 2026-08-15. The accuracy figure is a free comparison
  baseline for any classifier I write.
- **Work that is genuinely slow and genuinely visual.** Deep analysis of a full
  game takes minutes, so progress reporting, cancellation, and long-running
  task handles have honest uses rather than simulated ones. A board is worth
  drawing, so an interactive UI is not decoration.

Chess.com already ships Game Review. This is not a market gap and should not be
justified as one. The consumer is me; the oracle and the measurement are free;
that is the whole case.

## What this is not

**Not live coaching during a game.** The chess.com mobile app does not speak
MCP and will not — real-time engine assistance is cheating, which is why no such
integration exists anywhere. The loop is: play on the phone, the finished game
lands in the public archive, review it at the desk afterwards.

**Not a chess engine.** Stockfish is the engine. Nothing here re-derives
evaluation.

**Not a general chess tutor.** It explains positions that occurred in my games.
It does not teach openings from scratch or generate puzzles.

## The open question

Stockfish emits an evaluation and a principal variation. A human-usable
explanation requires reasons, and the gap between the two has to be bridged
somewhere. Two candidates, unresolved:

- **Server computes verified facts; the model only narrates them.** Derive
  attack maps, pins, forks, hanging material, and king-safety and pawn-structure
  deltas with `python-chess`, ship them as `structuredContent`, and give the
  model a fact list rather than a board. The model cannot invent chess because
  it was never asked to read a position. Cost: one detector per motif, so
  coverage is a dozen patterns where a coach has hundreds.

- **The model reasons over principal variations directly.** Much less code and
  unbounded coverage, including positions no detector anticipates. Cost: LLMs
  read FEN badly and will assert tactics that are not on the board.

The failure modes are opposite — shallow-but-correct against fluent-but-wrong —
and the first is exactly the complaint about chess.com's templated reviews, so
neither is obviously right.

**Resolve by measurement, not argument.** Run both over 50 of my own games and
count how often each produces a claim that is false about the position. Nothing
in the design gets committed before that number exists.

## Success criteria

- Every non-deprecated MCP surface in the 2026-07-28 revision is exercised by
  something with a real use, not a stub written to tick a box.
- Given a raw JSON-RPC exchange from this server, I can say what each field is
  for and what the SDK does with it, without looking it up.
- Reviews contain no false statements about the board. A review that says less
  and is correct beats a richer one that is not.
- My play improves. Rating is a noisy proxy over three months; a better one is
  whether the specific mistakes the reviews flag stop recurring.

## Protocol baseline

Targets MCP revision `2026-07-28`. Recorded here because most published
tutorials target `2025-06-18` and would teach a protocol that is already on a
removal clock — see the [deprecated features registry](https://modelcontextprotocol.io/specification/2026-07-28/deprecated).

Deprecated as of this revision, and therefore not to be adopted: Sampling,
Roots, Logging (all SEP-2577), OAuth Dynamic Client Registration, and the
HTTP+SSE transport. Removed outright: the `initialize` handshake, protocol-level
sessions and `Mcp-Session-Id`, `ping`, and `logging/setLevel` — MCP is now
stateless, with protocol version and client capabilities carried in `_meta` on
every request.

Surfaces to cover, with the intended use for each:

| Surface | Use |
|---|---|
| Tools | `search` / `execute` / `whoami` over the backend's schema; `review_game` for the long-running case |
| Resources + templates | `chess://game/{id}`, `chess://game/{id}/ply/{n}`, `chess://opening/{eco}` |
| Prompts | "review my last game", "drill this endgame" — user-invoked |
| `server/discover` | mandatory in this revision |
| Completion | autocomplete over my game IDs, ECO codes, opening names |
| Pagination | a year of blitz runs to thousands of games |
| Caching (`ttlMs`, `cacheScope`) | finished-game analysis is immutable; openings are `public`, my games `private` |
| Elicitation via MRTR | "three games Tuesday — which?"; "were you White?" |
| Progress | depth and ply during a long analysis |
| Cancellation | stop a search that is deep enough |
| `subscriptions/listen` | analysis backlog completing; `resourcesListChanged` on import |
| stdio + Streamable HTTP | both transports |
| Authorization | needs a source that requires it — see below |
| Tasks extension | full-game review is the long-running-handle case |
| Apps extension | interactive board, engine-line arrows, eval bar |

The chess.com public API needs no authentication, so it cannot exercise the
authorization surface. Lichess has a real OAuth 2.0 + PKCE flow and an
equivalent game archive; adding it as a second source is the way to cover
authorization honestly. Unverified so far — confirm the Lichess flow works
against this revision's client-registration rules before relying on it.

## Open questions beyond the fork

- Which client hosts the Apps extension UI during development, and does the
  local dev loop for a `ui://` resource work without publishing anything?
- Does a fixed Stockfish depth give reproducible evaluations across runs on
  this machine, or does time management make results vary? Reproducibility is
  assumed by the measurement plan above and has not been checked.
- What counts as a "critical moment" — an eval swing past a threshold, or
  something better? A threshold on centipawn loss is the obvious first cut and
  is known to misfire in winning and lost positions.
