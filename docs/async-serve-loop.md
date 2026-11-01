# Notes for an async serve loop

> Historical as of 2026-11-01: there is no MCP server; see
> [0002](0002-tauri-desktop-app-rust-backend.md). Kept for when MCP returns.

Decided 2026-08-18: the stdio serve loop is async. This is what that decision
buys, what it costs, and where to look for prior art. It deliberately stops
short of prescribing a structure — the point of hand-writing the server is to
arrive at one.

## What forces it

Two surfaces in `PROBLEM.md` are unimplementable on a synchronous loop, for
the same reason. A loop that reads a line, handles it to completion, and then
reads the next line is not reading stdin while a handler runs.

- **Cancellation.** `notifications/cancelled` arrives on that unread stdin. A
  sync server can only notice a cancel after finishing the work being
  cancelled, which is not cancellation.
- **Progress.** `notifications/progress` has to be emitted from inside a
  running handler, interleaved with whatever else is on the wire.

Both are in the surface table PROBLEM.md commits to covering, and a
depth-25 full-game analysis is exactly the case where they earn their place
rather than being simulated.

## Four problems it creates

Sequential code has none of these. Each is a genuine decision.

### 1. Reading stdin without blocking

`sys.stdin.readline()` blocks the event loop, so every concurrent handler
stops with it. Three ways out:

| Approach | Notes |
|---|---|
| `loop.connect_read_pipe` + `asyncio.StreamReader` | Raw asyncio. **Verified working on this machine** — `tmp/probe_async_stdin.py` reads a piped stdin and sees EOF. Unix-only in practice; Windows `ProactorEventLoop` does not support pipes for stdin. |
| `asyncio.to_thread` around blocking reads | Portable, costs a thread, and makes clean shutdown harder because a blocked read is not cancellable. |
| `anyio.wrap_file` | What the SDK does. Adds a dependency and an abstraction layer over the thing being learned. |

The SDK reaching for anyio is not evidence that you should. It supports
asyncio *and* trio; you support one event loop, and the extra layer sits
exactly on top of the mechanism goal 2 is about.

### 2. Two writers tear a frame

The moment more than one task can write to stdout, newline-delimited framing
is at risk: two interleaved writes produce one corrupt line and every message
after it is garbage. Nothing in the transport detects this — the client just
sees malformed JSON.

The structural fix in `mcp/server/stdio.py` (SDK 2.0.0, in `tmp/sdkprobe/`) is
a **single dedicated writer task**. Handlers never touch stdout; they put
messages on an in-memory stream, and one `stdout_writer` coroutine drains it:

```python
async def stdout_writer():
    async with write_stream_reader:
        async for session_message in write_stream_reader:
            json = session_message.message.model_dump_json(...)
            await stdout.write(json + "\n")
            await stdout.flush()
```

A lock around every write would also work. The queue makes the invariant
structural instead of a rule everyone has to remember.

### 3. A stray `print()` still corrupts the wire

The SDK's answer to this is worth reading in full — `_claim_fd`, lines
106–158. While serving, it points **fd 0 at `/dev/null` and fd 1 at stderr**,
and serves the real wire from a private duplicate obtained before the swap.

The consequence: a handler that calls `print()`, or a subprocess that writes
to its inherited stdout, physically cannot reach the wire. Their output goes
to stderr. Stockfish is a subprocess that chats on stdout, which makes this
directly relevant.

The comments explain the failure modes they hit getting there — issue #1933
(closing the wrapper destroyed `sys.stdout` for the whole process), and the
`closefd=False` note about a worker thread still blocked on a descriptor after
the transport exits. Both are the kind of thing you only learn by breaking it.

Whether you want this is a real question. It is defensive plumbing, not
protocol, and it is a lot of `os.dup2` for a server you control. Knowing the
trick exists matters more than adopting it.

### 4. Stockfish blocks

`chess.engine.SimpleEngine` is synchronous and would block the loop for the
whole search. python-chess ships a native asyncio protocol instead — verified
in the installed 1.11.2:

```python
chess.engine.popen_uci(command) -> (asyncio.SubprocessTransport, UciProtocol)
```

`UciProtocol` subclasses `asyncio.SubprocessProtocol`. Two methods matter and
they are not interchangeable:

- `await engine.analyse(board, limit)` — runs to completion, returns an
  `InfoDict`. Fine for a single position.
- `await engine.analysis(board, limit)` — returns an `AnalysisResult`, which
  is an **async-iterable handle** with a `stop()` and a `_finished` future.
  Its docstring: "Can be used to asynchronously iterate over information sent
  by the engine. Automatically stops the analysis when used as a context
  manager."

`analysis()` is the one that maps onto both surfaces. Iterating it yields
`InfoDict`s as the engine deepens — that is your progress notification, with
real depth numbers rather than a fake percentage. Calling `stop()` is your
cancellation. If you build on `analyse()` you will have to unpick it later.

Reproducibility still holds under this: the depth-20 determinism measured in
SESSION.md assumed `Threads 1` and a fresh hash table. Reusing one long-lived
engine process across analyses means the hash table is *not* fresh, and that
is enough to change node counts. Decide whether the engine is per-request or
per-server, and check the invariant again after.

## Reader and writer as separate tasks

SESSION.md asks whether the writer is a separate task from the first
milestone, on the grounds that retrofitting a queue means touching every reply
path. Reading the SDK's write path answers a slightly different question than
the one asked, and the difference is the whole decision.

**Every outbound message in the SDK passes through one line.** Not one module —
one statement, `jsonrpc_dispatcher.py:778`:

```python
await self._write_stream.send(SessionMessage(message=message, metadata=metadata))
```

That is the only `send` on `_write_stream` in the file. The dispatcher holds
progress notifications, error responses, results, and courtesy cancels, and all
of them funnel through it before the transport ever sees them. So the thing that
makes the retrofit cheap is not the writer task; it is that no reply path writes
output itself. Given one `async def send()`, swapping its body from "write to
stdout" to "put on a queue" is a change to one function. The premise that a
retrofit touches every reply path holds only if reply paths touch stdout, and
that is the property to establish now.

Three things follow from reading the transport rather than assuming what it
does:

- **The queue is unbuffered.** `create_context_streams[...](0)` bottoms out in
  `anyio.create_memory_object_stream(0)`, a rendezvous: a handler calling
  `send()` blocks until `stdout_writer` takes the message. So the write queue
  buys serialization, not decoupling — the common argument for a queue, that
  handlers should not wait on a slow stdout, is not what this code is doing.
- **Cancelled replies are suppressed above the writer.** "The cancelled request
  is never answered — the handler's eventual result or error is dropped, not
  written" is dispatcher policy. The writer task is not what enforces the spec's
  MUST NOT, so that rule is no argument for having one.
- **What the writer task does buy** is that stdout becomes unreachable from
  handler code by construction rather than by convention, and that "flush what
  is pending, then exit" becomes expressible at all.

### The decision

One task through `tools/list`, with every reply going through a single
`send()`. Add the writer task at the first milestone where two tasks can call
it — `tools/call` into Stockfish with progress, which is also the first
milestone that needs concurrent handlers for cancellation to mean anything.
Nothing before that point can produce two concurrent writers, so nothing before
it can tear a frame.

The reason to wait is diagnostic. SESSION.md lists four failures to expect from
the first milestone — block-buffered stdout, a stray `print()`, `indent=`
breaking framing, no exit on EOF — and each teaches a stdio rule by being
debugged. A writer task adds a second explanation for three of the four: when it
hangs, the cause is now either block buffering or a writer that never got
scheduled, and the lesson is confounded. Build the thing whose failures have one
cause each.

The argument against, which is real: EOF shutdown is one of those four failures,
and it is the one whose answer does not survive the change. A single-task loop
exits when the read returns empty; a two-task loop has to stop the reader, decide
whether pending writes drain, and tell the writer to finish — the drain-vs-abandon
question below. That discipline gets learned twice. Accepted, because the second
time is where in-flight work exists to reason about, and the question is
unanswerable without it.

## Shutdown, which is now ambiguous

Sequentially, EOF on stdin means "nothing more will arrive, exit." With
handlers in flight it splits into two behaviours and the spec does not pick
one:

- **Drain** — stop reading, let running tasks finish and reply, then exit.
- **Abandon** — cancel everything and exit now.

What the spec does say: servers SHOULD exit *promptly* on EOF, that this is
the primary and only portable graceful-shutdown signal, and that a client will
escalate to `SIGTERM` then `SIGKILL` if ignored. It also says in-flight
requests are simply lost on restart and the client can retry them, because the
protocol is stateless. That leans toward abandon, and toward a bounded grace
period rather than an unbounded drain, but it is an argument, not a rule.

Related, and easy to get wrong: the spec says a server MUST NOT send any
further messages for a cancelled request. So the task registry that lets a
cancel find its target also has to suppress the reply that task was about to
produce.

## What to read

In order, and all of it short:

1. `tmp/sdkprobe/mcp/server/stdio.py` — 218 lines, the whole transport. Note
   that `stdin_reader` sends *parse exceptions* into the stream rather than
   handling them, so the transport carries no error policy.
2. `tmp/sdkprobe/mcp/shared/inbound.py` — the validation ladder and the
   error-code mapping. A pure module by its own docstring: no I/O, no
   transport. It classifies a decoded body into a route or a rejection
   carrying `UNSUPPORTED_PROTOCOL_VERSION` / `METHOD_NOT_FOUND`, which is the
   first three milestones after `server/discover`. It does **not** invoke
   handlers.
2b. `tmp/sdkprobe/mcp/shared/jsonrpc_dispatcher.py` — where messages become
   handler invocations. Handler spawning (`_spawn`, line 671), the
   cancellation and progress wiring, and the single exception-to-wire
   boundary.
3. `tmp/sdkprobe/mcp/server/runner.py` — dispatch, and the
   `UNSUPPORTED_PROTOCOL_VERSION` path.
4. `.venv/lib/python3.13/site-packages/chess/engine.py`, class
   `AnalysisResult` at line 2703.

Read 1 before writing your loop and 2–3 after, so you have made the decisions
yourself before seeing someone else's.
