# MCP 2026-07-28: a working reference

Wire shapes for the revision this project targets, condensed from the
specification so you are not holding five browser tabs open while writing a
server.

Every JSON block below is quoted from the spec pages, not reconstructed from
memory. Where a page has not been read yet it says so rather than guessing —
see [Not yet read](#not-yet-read) at the end, and treat anything absent from
this document as unverified.

Fetched 2026-08-18 from `https://modelcontextprotocol.io/specification/2026-07-28/`.
The spec is versioned by URL path, so those links are stable; the `draft`
paths are not, and one of them is cited in the SDK (see
[Apps](#apps-extension)).

## Why not to trust a tutorial

Nearly every published MCP tutorial targets `2025-06-18`. Five things that
revision does are gone here, and code copied from one will not run:

| Gone | Replaced by |
|---|---|
| The `initialize` handshake | `_meta` on every request |
| Sessions and `Mcp-Session-Id` | Nothing — the protocol is stateless |
| `ping` | Nothing |
| `logging/setLevel` and the Logging surface | Deprecated (SEP-2577) |
| HTTP+SSE transport | Streamable HTTP |

Also deprecated as of this revision, and therefore not to be adopted:
Sampling, Roots, and OAuth Dynamic Client Registration.

The MCP Python SDK draws the same line. From `mcp_types/version.py` in
version 2.0.0:

```python
HANDSHAKE_PROTOCOL_VERSIONS = ("2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25")
MODERN_PROTOCOL_VERSIONS    = ("2026-07-28",)
```

"Modern" is the SDK's word for the per-request-envelope era. When you read SDK
source, that split is the first thing to check — a code path guarded by
`is_version_at_least(v, "2025-11-25")` is handshake-era machinery that does not
apply to you.

## The envelope

Requests carry their metadata inline, in `params._meta`, under keys namespaced
`io.modelcontextprotocol/`. This is required on **every** request, not just a
first one:

```json
{
  "jsonrpc": "2.0",
  "id": "discover-1",
  "method": "server/discover",
  "params": {
    "_meta": {
      "io.modelcontextprotocol/protocolVersion": "2026-07-28",
      "io.modelcontextprotocol/clientInfo": {
        "name": "ExampleClient",
        "version": "1.0.0"
      },
      "io.modelcontextprotocol/clientCapabilities": {}
    }
  }
}
```

The `tools` and `resources` pages both omit `_meta` from their examples for
brevity and say so in a note. Do not read those examples as showing an
optional field.

Every *result* carries `resultType`. It is `"complete"` for an ordinary reply
and `"input_required"` when the server needs another round trip — that second
value is the MRTR mechanism, and it is what replaced every server-initiated
request. On stdio in particular the server **MUST NOT** write JSON-RPC
requests to stdout at all; anything the server needs from the client comes
back as an `InputRequiredResult` on a reply the client already asked for.

Two consequences that shape a server, not just its parsing:

- There is no point at which you can validate a client once and trust it
  afterwards. Version checking is per-request because there is nothing else.
- There is no protocol-level place to keep state between calls. The tools page
  is explicit that a server needing continuity should return an opaque handle
  from one tool and accept it as an argument to the next — and that from the
  wire's perspective a handle is just a string.

## stdio transport

<https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio>

Framing is newline-delimited JSON. One complete JSON-RPC message per line.
There is no `Content-Length` header layer — that belonged to LSP and to the
removed HTTP+SSE transport.

The rules, quoted:

- Messages are delimited by newlines, and **MUST NOT** contain embedded newlines.
- The server **MUST NOT** write anything to its `stdout` that is not a valid MCP message.
- The server **MAY** write UTF-8 strings to `stderr` for any logging purposes.
- The client **MAY** capture, forward, or ignore the server's `stderr` output and **SHOULD NOT** assume `stderr` output indicates error conditions.
- The client **MUST NOT** write JSON-RPC *responses*.
- The server **MUST NOT** write JSON-RPC *requests* to `stdout`.

Two practical traps in that list. A stray `print()` in a handler corrupts the
stream for every message after it. And `json.dumps(..., indent=2)` produces
embedded newlines, so an indented message breaks framing — the default
non-indented output is not a style preference here.

**Shutdown.** The client closes the server's stdin and waits; the server
**SHOULD** exit promptly on EOF. The spec calls this "the primary
graceful-shutdown signal and the only portable one." Escalation to `SIGTERM`
then `SIGKILL` is the client's fallback when it is ignored.

**Unexpected termination.** The client SHOULD just restart the process.
Because the protocol is stateless, in-flight requests are simply lost and can
be retried against the fresh process — but active `subscriptions/listen`
streams must be re-established.

**Cancellation.** `notifications/cancelled` referencing the request id, on the
same single shared channel. There is no per-request stream to close. Servers
**SHOULD** stop work as soon as practical and **MUST NOT** send any further
messages for that request.

> This is the constraint that decides whether your serve loop can be
> synchronous. A synchronous loop is not reading stdin while a handler runs,
> so a cancel cannot be observed until the work it cancels has already
> finished. Same for emitting progress mid-search. Deferring async is
> defensible; not noticing is not.

**Server-to-client message kinds on stdout**, and there are exactly three:
responses correlated by `id`; notifications relating to an in-flight request
such as `notifications/progress`; and notifications for an active
`subscriptions/listen`, which the client correlates via
`_meta["io.modelcontextprotocol/subscriptionId"]`.

## server/discover

<https://modelcontextprotocol.io/specification/2026-07-28/server/discover>

Servers **MUST** implement it. Clients need not call it.

Request is the bare envelope — no params beyond `_meta`. Response:

```json
{
  "jsonrpc": "2.0",
  "id": "discover-1",
  "result": {
    "resultType": "complete",
    "supportedVersions": ["2026-07-28"],
    "capabilities": {
      "tools": {},
      "resources": {}
    },
    "_meta": {
      "io.modelcontextprotocol/serverInfo": {
        "name": "ExampleServer",
        "version": "1.0.0"
      }
    },
    "instructions": "This server provides weather and resource utilities.",
    "ttlMs": 3600000,
    "cacheScope": "public"
  }
}
```

`instructions` is optional natural-language guidance for the model on how to
use the server. `serverInfo` is self-reported and unverified — the spec notes
clients **SHOULD NOT** change behavior based on it or use it for security
decisions, which also means it is useless for disambiguating two servers that
both expose a `search` tool.

**The stdio fallback probe.** There is no HTTP status code on stdio to drive
era detection, so a client supporting both eras SHOULD send `server/discover`
first. Three outcomes:

1. A `DiscoverResult` → modern server, pick a version from `supportedVersions`.
2. A recognized modern error such as `UnsupportedProtocolVersionError` →
   modern server, wrong version. Use its advertised `supported` list. **Do not**
   fall back to `initialize`.
3. Any other error, or a timeout → legacy server, fall back to `initialize`.

The fallback **MUST NOT** be keyed to one error code: legacy servers answer
unknown pre-`initialize` requests with implementation-defined errors, commonly
`-32601` or `-32602`, or not at all.

## Tools

<https://modelcontextprotocol.io/specification/2026-07-28/server/tools>

Capability declaration. `listChanged` says whether you will emit change
notifications:

```json
{ "capabilities": { "tools": { "listChanged": true } } }
```

The tool set **MUST NOT** vary per-connection or as a side effect of other
requests, though it **MAY** vary by the authorization presented on the request
— credentials are per-request input, not connection state. Return tools in a
deterministic order; the spec's stated reason is client caching and LLM prompt
cache hit rates.

### tools/list

Supports pagination and caching.

```json
{ "jsonrpc": "2.0", "id": 1, "method": "tools/list",
  "params": { "cursor": "optional-cursor-value" } }
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "resultType": "complete",
    "tools": [
      {
        "name": "get_weather",
        "title": "Weather Information Provider",
        "description": "Get current weather information for a location",
        "inputSchema": {
          "type": "object",
          "properties": {
            "location": { "type": "string", "description": "City name or zip code" }
          },
          "required": ["location"]
        },
        "icons": [
          { "src": "https://example.com/weather-icon.png",
            "mimeType": "image/png", "sizes": ["48x48"] }
        ]
      }
    ],
    "nextCursor": "next-page-cursor",
    "ttlMs": 300000,
    "cacheScope": "public"
  }
}
```

### tools/call

```json
{ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
  "params": { "name": "get_weather", "arguments": { "location": "New York" } } }
```

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "resultType": "complete",
    "content": [
      { "type": "text",
        "text": "Current weather in New York:\nTemperature: 72°F\nConditions: Partly cloudy" }
    ],
    "isError": false
  }
}
```

### Tool definition fields

- `name` — unique within the server. 1–128 chars, case-sensitive, and the only
  characters SHOULD be `A-Za-z0-9_-.`
- `title` — optional display name
- `description` — human-readable
- `icons` — optional, for UI
- `inputSchema` — JSON Schema, **MUST** be a valid schema object and not
  `null`. Defaults to draft 2020-12 with no `$schema`. For a tool with no
  parameters the recommended form is `{"type": "object", "additionalProperties": false}`
- `outputSchema` — optional. If present, servers **MUST** produce conforming
  `structuredContent` and clients SHOULD validate it
- `annotations` — optional behavior hints. Clients **MUST** treat these as
  untrusted unless the server is trusted

### Content types in a result

`text`, `image` (base64 `data` + `mimeType`), `audio`, `resource_link` (a URI
the client can fetch or subscribe to), and `resource` (an embedded
`ResourceContents`). All of them accept the same `annotations` block resources
use — `audience`, `priority`, `lastModified`.

`structuredContent` is a separate top-level field holding any JSON value. For
backwards compatibility a tool returning structured content SHOULD *also*
serialize it into a text block:

```json
{
  "jsonrpc": "2.0", "id": 5,
  "result": {
    "resultType": "complete",
    "content": [
      { "type": "text",
        "text": "{\"temperature\": 22.5, \"conditions\": \"Partly cloudy\", \"humidity\": 65}" }
    ],
    "structuredContent": { "temperature": 22.5, "conditions": "Partly cloudy", "humidity": 65 }
  }
}
```

The spec adds a note worth keeping straight: `structuredContent` is
server-produced result data and has nothing to do with LLM "structured
outputs" in the schema-constrained-generation sense.

### The two error mechanisms

This distinction is the one most worth getting right, because it decides what
the model gets to see.

**Protocol errors** — unknown tool, malformed request, server fault. Returned
as a JSON-RPC error. Clients MAY show these to the model, and the spec says
they are "less likely to result in successful recovery."

```json
{ "jsonrpc": "2.0", "id": 3,
  "error": { "code": -32602, "message": "Unknown tool: invalid_tool_name" } }
```

**Tool execution errors** — API failure, input out of range, business logic.
Returned as a *successful* JSON-RPC response with `isError: true`. Clients
SHOULD hand these to the model so it can self-correct.

```json
{ "jsonrpc": "2.0", "id": 4,
  "result": {
    "resultType": "complete",
    "content": [
      { "type": "text",
        "text": "Invalid departure date: must be in the future. Current date is 08/08/2025." }
    ],
    "isError": true
  } }
```

For this project: an illegal FEN is a *tool execution* error — the model can
fix it. A `tools/call` for a tool that does not exist is a protocol error.

### InputRequiredResult on a tool call

A server MAY answer `tools/call` with `resultType: "input_required"` instead of
completing:

```json
{
  "jsonrpc": "2.0", "id": 2,
  "result": {
    "resultType": "input_required",
    "inputRequests": {
      "github_login": {
        "method": "elicitation/create",
        "params": {
          "mode": "form",
          "message": "Please provide your GitHub username",
          "requestedSchema": {
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"]
          }
        }
      }
    },
    "requestState": "eyJsb2NhdGlvbiI6Ik5ldyBZb3JrIn0..."
  }
}
```

The client retries with `inputResponses` keyed by the same name, echoing
`requestState` if one was given:

```json
{ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
  "params": {
    "name": "get_weather",
    "arguments": { "location": "New York" },
    "inputResponses": { "github_login": { "action": "accept", "content": { "name": "octocat" } } },
    "requestState": "eyJsb2NhdGlvbiI6Ik5ldyBZb3JrIn0..."
  } }
```

The JSON-RPC `id` **MUST** differ between the initial request and the retry.
`requestState` is how a stateless server carries partial work across the round
trip — it is opaque to the client, and choosing what goes in it is a real
design decision, not bookkeeping.

Read the MRTR page before relying on any of this; see
[Not yet read](#not-yet-read).

## Resources

<https://modelcontextprotocol.io/specification/2026-07-28/server/resources>

```json
{ "capabilities": { "resources": { "listChanged": true, "subscribe": true } } }
```

Both sub-features are independent and both optional; a server supporting
neither declares `"resources": {}`.

### resources/list

```json
{ "jsonrpc": "2.0", "id": 1, "method": "resources/list",
  "params": { "cursor": "optional-cursor-value" } }
```

```json
{
  "jsonrpc": "2.0", "id": 1,
  "result": {
    "resultType": "complete",
    "resources": [
      { "uri": "file:///project/src/main.rs",
        "name": "main.rs",
        "title": "Rust Software Application Main File",
        "description": "Primary application entry point",
        "mimeType": "text/x-rust",
        "icons": [ { "src": "https://example.com/rust-file-icon.png",
                     "mimeType": "image/png", "sizes": ["48x48"] } ] }
    ],
    "nextCursor": "next-page-cursor",
    "ttlMs": 300000,
    "cacheScope": "public"
  }
}
```

### resources/read

```json
{ "jsonrpc": "2.0", "id": 2, "method": "resources/read",
  "params": { "uri": "file:///project/src/main.rs" } }
```

```json
{
  "jsonrpc": "2.0", "id": 2,
  "result": {
    "resultType": "complete",
    "contents": [
      { "uri": "file:///project/src/main.rs",
        "mimeType": "text/x-rust",
        "text": "fn main() {\n    println!(\"Hello world!\");\n}" }
    ],
    "ttlMs": 60000,
    "cacheScope": "private"
  }
}
```

A read MAY return several contents — the spec's example is a directory
resource returning the files under it. Binary content uses `blob` with base64
instead of `text`.

### resources/templates/list

```json
{
  "jsonrpc": "2.0", "id": 3,
  "result": {
    "resultType": "complete",
    "resourceTemplates": [
      { "uriTemplate": "file:///{path}",
        "name": "Project Files",
        "title": "📁 Project Files",
        "description": "Access files in the project directory",
        "mimeType": "application/octet-stream" }
    ],
    "nextCursor": "next-page-cursor",
    "ttlMs": 300000,
    "cacheScope": "public"
  }
}
```

Templates are RFC 6570 URI templates, and their arguments are what the
completion API autocompletes over. That is the hook for the "autocomplete my
game IDs and ECO codes" item in PROBLEM.md.

### Error handling — different from tools

A missing resource is a JSON-RPC error with code **`-32602`**, not a result
with `isError`. Internal faults SHOULD be `-32603`. Clients SHOULD also accept
`-32002` because earlier revisions used it.

```json
{ "jsonrpc": "2.0", "id": 5,
  "error": { "code": -32602, "message": "Resource not found",
             "data": { "uri": "file:///nonexistent.txt" } } }
```

And explicitly: servers **MUST NOT** return an empty `contents` array for a
non-existent resource, because an empty array is ambiguous between "exists,
has no content" and "does not exist."

### URI schemes

`https://` only when the client can fetch it directly without going through
the server. `file://` for filesystem-*like* resources, which need not map to a
real filesystem. `git://`. Custom schemes are fine and must satisfy RFC 3986 —
which is what `chess://game/{id}` will be.

## Error codes

| Code | Meaning | Source |
|---|---|---|
| `-32700` | Parse error | JSON-RPC 2.0 |
| `-32600` | Invalid request | JSON-RPC 2.0 |
| `-32601` | Method not found | JSON-RPC 2.0 |
| `-32602` | Invalid params; also **resource not found** | JSON-RPC 2.0 + MCP |
| `-32603` | Internal error | JSON-RPC 2.0 |
| `-32022` | `UnsupportedProtocolVersionError` | MCP |
| `-32002` | Legacy resource-not-found; accept, do not emit | MCP, historical |

`-32022` is from the SDK source (`mcp/client/session.py`, version 2.0.0),
which documents it as the code triggering a version renegotiation. The
spec's schema page has not been read directly — see below.

## Not yet read

Do not treat this document as covering these. Each is a surface PROBLEM.md
commits to exercising:

| Page | Needed for |
|---|---|
| `basic/patterns/mrtr` | Elicitation. Read first — it drives the shape of every server-initiated interaction |
| `basic/patterns/subscriptions` | `subscriptions/listen`, `resourcesListChanged` |
| `server/utilities/caching` | `ttlMs`, `cacheScope` semantics beyond the field names |
| `server/utilities/pagination` | Cursor semantics and opacity rules |
| `server/utilities/completion` | Autocomplete over game IDs and ECO codes |
| `basic/patterns/cancellation` | Full rules beyond the stdio summary above |
| `basic/index#meta` | The authoritative `_meta` field list |
| `basic/versioning` | Negotiation flow and the era compatibility matrix |
| `schema` | Authoritative types; would confirm `-32022` and `resultType` values |
| Tasks extension | Long-running full-game review |
| Apps extension | The interactive board |
| `basic/transports/streamable-http` | The second transport |
| Authorization | Lichess OAuth, and whether its flow survives DCR's deprecation |

### Apps extension

Not read, but one thing found while checking the SDK. `mcp/server/apps.py` in
SDK 2.0.0 cites
`https://modelcontextprotocol.io/specification/draft/extensions/apps` — a
**draft** path, not a pinned revision. The wire format may still move. It is
also the thinnest surface in the SDK at a single module, against
implementations spread across many files for tools and resources.
