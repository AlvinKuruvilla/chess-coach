# Session notes

Scratch log for this project: what to do next, and incidental problems found
along the way that are not worth stopping for. See PROBLEM.md for the problem
statement and the ranked goals.

Status as of 2026-08-30: **the server works.** Two tools over stdio on the SDK
(`mcp==2.1.1`), verified end to end against real corpus games. A 61-ply game
reviews in 13.6s at depth 16 and emits 62 progress notifications. Nothing is
committed yet — the repo still has zero commits.

## What exists

- `src/chess_coach/analysis.py` — evaluates every position in a game once,
  which yields the centipawn loss, the move that should have been played, and
  the refutation. `consequence_of` plays the refutation forward and diffs the
  board into one of four kinds: `material`, `mate`, `trade_neutral`,
  `positional`. The last two exist so a narrator is told the captures are not
  the reason, rather than being handed a list to embroider.
- `src/chess_coach/main.py` — `MCPServer` with `review_game(pgn, depth,
  threshold_cp)` and `analyse_position(fen, depth)`. Progress via
  `ctx.report_progress`; Stockfish located at startup so a missing binary
  fails with a sentence rather than inside the first call.
- `tests/` — 9 passing. `consequence_of`'s four kinds are engine-free; the
  game-level tests use Stockfish at depth 6-8 and check bookkeeping (which
  side erred, that the loss is positive for a Black mistake) rather than the
  engine's judgement.
- `tmp/client.py` — a stdio client that holds the pipe open. Needed: see below.

Verified by running, not by reading:

```
8... d6   -331cp  best O-O   material  swing +9  ['h8', 'h7']
          Qxh8+ Kd7 Qxh7 Nc6 Nc3
```

## Three things the wire taught, that reading would not have

- **A single piped line loses the reply.** `echo '...' | uv run chess_coach`
  closes stdin immediately, the transport sees EOF while the handler is still
  running, and the SDK **abandons** in-flight work — the client gets
  `-32000 Connection closed`. That settles the drain-vs-abandon question in
  `docs/async-serve-loop.md` as observed behaviour: abandon. Any client must
  hold stdin open until the reply arrives.
- **`progressToken` is a bare key.** It lives at `params._meta.progressToken`,
  *not* under the `io.modelcontextprotocol/` namespace that `protocolVersion`,
  `clientInfo`, and `clientCapabilities` use. Without it the server emits no
  progress at all and looks broken.
- **A delivered checkmate read as a 2000cp blunder by the winner.** Stockfish
  scores a mated position as `Mate(0)`, and zero carries no sign. Terminal
  positions are now decided from the board and never handed to the engine.

## Start here next session

Architecture as of 2026-11-01: a Tauri desktop app with a Rust backend; see
[docs/0002](docs/0002-tauri-desktop-app-rust-backend.md). Feature order agreed
2026-11-01:

1. ~~**Mistakes by winning chances, not centipawns.**~~ Done 2026-11-01; see
   `docs/win-curve.md`. Open: the fit method did not reproduce Lichess's
   constant on 2300+ games (0.0051 vs 0.0037), unexplained.
2. ~~**Review in the background after sync.**~~ Done 2026-11-01.
3. ~~**The "find the move" trainer, fed by your own mistakes.**~~ First version
   done 2026-11-01 (Practice tab). Not yet seen running by Claude; no
   screenshot tool was connected. Decided
   2026-11-01 after research (summarised in the session; key sources: the
   2026 Psychological Science study of 44,213 chess.com players, study-type
   activities 3.61x more efficient than play; Lichess "Learn from your
   mistakes"). Show the position before a mistake, you move, the engine
   judges it by expected score lost; a miss hides the answer and prompts
   checks/captures/threats for one retry, then reveals the best move, the
   refutation and the cost. Only `material` and `mate` mistakes, where a
   best move is clear. Missed positions come back on a spaced schedule.
4. **What-if exploration** -- the trainer's judging without the retry, from
   any position in a finished game. Needs a persistent engine and a
   variation tree in the playback state.
5. **Why the reply works.** Detectors over the refutation for a loose piece,
   fork, pin, overloaded defender, back rank -- tier 2 in
   `docs/experiment-metric.md`. They make the trainer's "why" real; until
   then it is the cost and the refutation line only.
6. **LLM coaching over those facts**, never over the raw board. Check it by
   counting false claims over the reviewed games before trusting it.
7. **Practice games with coach mode**: play a weakened engine, and the
   trainer steps in before a mistake instead of after. Opponent: Stockfish
   first (`UCI_Elo` floors at 1320 in Stockfish 19; below that only `Skill
   Level`, which blunders unlike a human). Maia-2 is human-like but needs
   Python/PyTorch and buckets every rating under 1100 together
   (CSSLab/maia2 `utils.py` at 67bee6c).
8. **Patterns across games.** Needs more games than the 8 rated ones so far.

Later, agreed 2026-11-01:

- **In-progress daily games, listed only.** `GET /pub/player/{user}/games`
  returns ongoing daily games with PGN, FEN, whose turn and `move_by`. Show
  opponent, whose move, time left, and a link to chess.com -- no board
  analysis. Live games in progress are not in the public API at all.
- **Fair-play guard.** The backend refuses to analyse any position from a game
  still in progress. Engine help on an ongoing game is cheating under
  chess.com's rules, daily included, whoever sends the move.
- **Review on finish.** When an in-progress game ends, review it at once so
  it is ready on next open. Optionally precompute while it runs and reveal
  only after chess.com marks it finished -- only if reviews feel slow.
- **GitHub Actions on Node 20.** `actions/checkout@v4` and `setup-node@v4`
  draw a deprecation warning; bump to their current majors.

Claude's standing role on this project, as of 2026-08-30: build it and show it
running.

## Blocked on input

- ~~**chess.com username.**~~ `alvinkuruvilla02`, confirmed against
  `api.chess.com/pub/player/alvinkuruvilla02` on 2026-08-18.

- **A corpus.** The account has no usable one, which invalidates three
  premises in PROBLEM.md. Verified 2026-08-18: one monthly archive, `2026/08`,
  holding three games — all dated 2026.08.13, all `rated=false`, all against
  `Coach-David` (a chess.com bot), all won by checkmate.

  - "50 of my own games" is not available; there are three.
  - "chess.com's own accuracy score for both players", called a free
    comparison baseline, is **absent** from all three. The `accuracies` key
    appears only once a Game Review has been run on a game.
  - Every game is `time_class: daily`. "A year of blitz runs to thousands of
    games", the stated justification for exercising pagination, is false for
    this account — pagination will never trigger on this data.

  The API behaves exactly as PROBLEM.md documents; the account is empty.
  Resolved for steps 1-5 by the Lichess open database below. Still open for
  goal 3, which no public dataset can serve — "did my play improve" needs my
  games, accumulated by playing them.

## Next steps, in order

1. ~~**Settle the SDK layer.**~~ **Reversed 2026-08-29: build on the SDK
   now.** Alvin's call, made against the argument below rather than in
   ignorance of it. What it costs is goal 2's second half — the SDK writes the
   framing, the dispatch, the validation, and the error codes, so none of that
   gets learned by having to make it work.

   Partial recovery, if wanted later: stdio is a pipe, so a `tee` between
   client and server yields the raw wire trace. That still serves the success
   criterion "given a raw JSON-RPC exchange from this server, I can say what
   each field is for" — reading traces, rather than producing them.

   The original decision, kept because its argument is still the argument
   against the current one. Decided 2026-08-18: hand-write first, then
   low-level SDK. A bare JSON-RPC server — `server/discover`, one tool, one
   resource, no SDK — against the 2026-07-28 stateless envelope, then move onto
   the SDK's low-level server API having seen the wire format. `MCPServer` and
   the decorator layer stay off the table.

   Rejected: building on the decorator layer and reading the SDK source
   separately to recover the mechanism. Its pitch is that you write no JSON
   Schema, no request parsing, no validation, and no protocol handling — which
   is precisely the material goal 2 is about, so the convenience removes the
   thing being learned. Reading source is not a substitute for having had to
   make the framing work.

   `mcp/server/runner.py` and `mcp/shared/inbound.py` are the reference for
   what dispatch and inbound framing are meant to look like.

1b. ~~**Serve loop is async.**~~ Moot as of 2026-08-29 — the SDK's loop is
   async and already separates reader from writer. The measured facts in
   `docs/async-serve-loop.md` stay true and stay useful for reading traces;
   only section 4, on the engine, is still work you have to do. Original note
   follows. Decided 2026-08-18. Forced by cancellation and
   progress: a sync loop is not reading stdin while a handler runs, so
   `notifications/cancelled` cannot be observed until the work it cancels has
   finished. See `docs/async-serve-loop.md` for the four problems this
   creates, what the SDK's `mcp/server/stdio.py` does about each, and the
   python-chess async engine API. Open sub-decisions recorded there: raw
   asyncio vs anyio, whether to adopt the fd-diversion trick, per-request vs
   per-server engine process, and drain-vs-abandon on EOF.

2. **Read the spec pages that were only skimmed.** Wire shapes for the pages
   read so far are in `docs/mcp-2026-07-28-reference.md`, quoted verbatim and
   pinned to the revision, with a "Not yet read" table at the end that is the
   live version of this step.

   Read: `server/discover`, `basic/transports/stdio`, `server/tools`,
   `server/resources`. Enough to write the first bare server.

   Remaining, MRTR first because it replaces every server-initiated request:
   `basic/patterns/mrtr`, `basic/patterns/subscriptions`,
   `basic/patterns/cancellation`, `server/utilities/caching`,
   `server/utilities/pagination`, `server/utilities/completion`,
   `basic/index#meta`, `basic/versioning`, `schema`, the Tasks and Apps
   extensions, `basic/transports/streamable-http`, and Authorization.

3. ~~**Check Stockfish depth reproducibility.**~~ Answered 2026-08-18: yes,
   bit-exact. Five depth-20 searches on Kiwipete
   (`r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1`)
   returned identical score (`cp -167`), bestmove (`d5e6`), and node count
   (673,556). Identical node counts mean the search tree is identical, not
   merely the conclusion. Holds under the conditions the experiment will run
   in — Stockfish's default `Threads 1`, and a fresh process per position so
   the hash table starts empty. Raising `Threads` forfeits this.

   Do not test this by piping `go depth N` followed by `quit`: Stockfish acts
   on `quit` immediately, emits no `info depth` lines, and returns a fallback
   move, which looks like a stable result and is not one.

4. **Fix the experiment's metric.** Designed 2026-08-29 in
   `docs/experiment-metric.md`; four open decisions listed at its end, all of
   which want settling before step 5 runs.

   Four counts, not one: false claims (gate), mechanism recall against the
   refutation's consequence (rank), true claims outside that denominator (where
   unbounded coverage would show), and unadjudicable claims (reported, unscored).

   Two numbers came out of the corpus while designing it. 6.7 mistakes per game
   at >= 200cp in the 400-999 band, so 50 games is ~335 scored positions. And
   83% of those mistakes have a mechanically-derivable consequence, measured over
   150 sampled positions at Stockfish depth 18 — the other 17% are positional and
   fall out of the recall denominator.

   The design has a consequence for the detector arm: it must ship a
   PV-consequence detector — what the best reply wins, from which square, by what
   relation — or the second axis picks the LLM arm by construction, mirroring the
   flaw it was written to fix.

5. **Run the false-explanation experiment.** 50 games from the Lichess band
   sample rather than my own — see the corpus section. Both bridging approaches
   from PROBLEM.md, scored on both axes from step 4. This settles the
   server-facts-vs-LLM-reasoning fork. No design gets committed before the
   numbers exist.

## Deferred and unverified

Recorded so they are not rediscovered later. None of these block the steps
above.

- **Lichess OAuth is unverified.** It is the plan for covering the
  authorization surface, since chess.com's public API needs no auth. Whether
  its flow satisfies this revision's client-registration rules is unchecked —
  Dynamic Client Registration is deprecated in favour of Client ID Metadata
  Documents, and Lichess may only support the deprecated path. If so, decide
  whether covering authorization against a deprecated mechanism is worth
  anything for goal 1.
- **Apps extension dev loop unknown.** Unclear which client to develop the
  `ui://` board against, and whether iterating on it locally requires
  publishing anything. Claude Desktop is listed as supporting the extension.
  Of every surface in the PROBLEM.md table this is the thinnest in the SDK —
  one module, `mcp/server/apps.py` — and its docstring cites the *draft* spec
  URL rather than a pinned revision, so the wire format may still move.
- **"Critical moment" is undefined.** A centipawn-loss threshold is the obvious
  first cut and is known to misfire in already-winning and already-lost
  positions, where a large eval swing changes nothing about the result.
- **Project name is a placeholder.** `chess-coach` was chosen as a plain
  descriptive directory name. Rename whenever.
- ~~**Python 3.14 support.**~~ Answered 2026-08-18: MCP Python SDK 2.0.0
  installs on 3.14.7 with no complaint. It also speaks `2026-07-28` natively —
  `mcp_types/version.py` splits `HANDSHAKE_PROTOCOL_VERSIONS` (through
  `2025-11-25`) from `MODERN_PROTOCOL_VERSIONS = ("2026-07-28",)`, the
  stateless per-request envelope. Every surface in the PROBLEM.md table has an
  implementation: `server/discover`, `tasks/`, elicitation, `completion/complete`,
  `subscriptions/listen`, `ttlMs`/`cacheScope`, pagination cursors,
  `progressToken`, `notifications/cancelled`.

## Corpus: the Lichess open database

Settled 2026-08-18. <https://database.lichess.org/>, CC0, monthly PGN dumps of
every rated game from 2013-01 to present. This replaces chess.com as the corpus
for everything except goal 3.

Measured, not recalled — one HTTP range request for the first 63 MB of
`lichess_db_standard_rated_2026-07.pgn.zst` (0.2% of a 29 GB file), decoded
through `zstd -dc`, which handles the truncated tail:

- 193,486 games in the sample.
- Elo spread covers the beginner band properly: 5.8% of player-slots under
  1000, 942 slots in 400-599. Peak is 1600-1799 at 19.4%.
- **10.4% of games carry inline `[%eval ...]`** — per-ply Stockfish evaluations
  already computed.
- Both players in 400-999 **and** `%eval` present: **1,269 games**. The 50-game
  experiment is 25x oversupplied by a 63 MB download.

Tags carried per game: `WhiteElo`, `BlackElo`, `ECO`, `Opening`, `TimeControl`,
`Termination`, plus the Lichess game URL as `Site`.

Why this beats the chess.com archive on every axis except ownership:

- **Right population, selectable.** PROBLEM.md's objection to somebody else's
  games is about population — explanations fail differently on a 400-rated
  hung queen than on a 2000-rated positional error. Filtering on `WhiteElo`
  and `BlackElo` addresses that directly, which an account archive cannot.
- **A better free baseline.** `%eval` is a per-ply evaluation, strictly more
  than the single `accuracies` figure PROBLEM.md was counting on — and that
  figure turned out to be absent from the chess.com games anyway.
- **Pagination becomes real.** 193k games in 0.2% of one month, against three
  daily games in the chess.com account.

Do not use the 2013 months for the experiment, cheap as they are
(`2013-01.pgn.zst` is 17.7 MB, 121,332 games). Early Lichess skewed strong:
375 player-slots under 1000 in the whole month, and 2 under 800. Fine for
development fixtures, wrong population for the measurement.

Keep this out of git — `tmp/` is already ignored.

## PROBLEM.md drift

- The goal 2 note calls the SDK-layer question "unresolved"; it was decided 2026-08-29
  (build on the SDK).
- "Open questions beyond the fork" still lists Stockfish depth reproducibility as
  unchecked; it was answered 2026-08-18 (bit-exact).
- "50 of my own games" and the chess.com accuracy baseline are stale; the corpus is the
  Lichess band sample.
- Goals 1-2 (MCP specification coverage, reading the SDK) are paused: there is
  no MCP server since 2026-11-01. The "Protocol baseline" section and its
  surface table describe nothing that exists.
- "One application, not a standalone MCP server" points at docs/0001, which is
  superseded by docs/0002; the backend/web-UI/MCP split it describes is gone.
- "The consumer is me" now holds more strongly: the chess.com account has 11
  games (8 rated, all daily) as of 2026-11-01, up from 3.

## Found 2026-11-01

- **PROBLEM.md still describes the MCP-first design**; see "PROBLEM.md drift".
  ADR 0001 is marked superseded and the two MCP docs are marked historical.
- **`docs/experiment-metric.md` still sizes the experiment at a 200cp
  threshold** (6.7 mistakes per game, ~335 positions). The app now classifies
  by expected score (`docs/win-curve.md`); decide which the experiment uses
  before running it, since the counts will differ.
- **Rust terminal detection is narrower than python-chess's.** shakmaty's
  `outcome` has no fivefold-repetition or 75-move rule, so those positions are
  searched instead of scored 0 directly (`terminal_cp` in `analysis.rs`).
- **Engine tests pass silently without Stockfish.** They return early rather
  than being marked ignored; CI installs Stockfish so they run there.
- **Single-position evaluation was dropped in the port.** `analyse_position`
  had no caller; the board view will want it back for exploring a line.
- **Changing the chess.com account is not supported.** The username is set
  once on first run; the `archives` table is not keyed by user, so a second
  account would skip months the first one fetched.
- **Frontend types are hand-copied** from the Rust serde types
  (`app/src/api.ts`). `ts-rs` or `tauri-specta` could generate them.
- **Trainer promotes to a queen only** (`Board.tsx`). A drill whose answer is
  an under-promotion cannot be solved.
- **Trainer judging starts a Stockfish process per move tried** (~1s at depth
  16). Fine for one move at a time; the persistent engine planned for what-if
  would remove it.
- **Engine views shift between searches**, so a move can score slightly above
  the "best" one; the trainer clamps the loss at zero.
- **`React.StrictMode` runs the launch sync twice in development.** Harmless
  (imports are idempotent) but makes two requests to chess.com.
