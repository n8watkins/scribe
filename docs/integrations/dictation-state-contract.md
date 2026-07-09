# Scribe Dictation-State Interface - Wire Contract (v1)

Status: FROZEN for the v1 (`schemaVersion: 1`) surface.
Producer: the Scribe desktop app (Tauri backend).
Audience: any external tool that wants to read Scribe's live dictation state.

This document is the authoritative, transport-level contract.
It is precise enough to build a consumer against without reading Scribe's Rust.
The canonical design rationale lives in the design proposal; this file pins the wire format.

Scribe OFFERS this interface to any tool.
Nothing here is specific to any single consumer.
A shell script, a text-to-speech daemon, a notification manager, or an AI agent all consume it the same way.

## 0. Versioning and stability

- Three independent integers, each starting at `1`, version the three payloads: the state snapshot (`schemaVersion`), the discovery file (`schemaVersion`), and the status file (`schemaVersion`).
- A version is bumped ONLY on a breaking change to that payload.
- Additive, backward-compatible fields may appear WITHOUT a version bump.
  Consumers MUST ignore unknown fields and MUST NOT fail on their presence.
- Consumers SHOULD reject a payload whose major `schemaVersion` is greater than the one they were written for.
- The path prefix `/v1/` on the HTTP routes tracks the snapshot `schemaVersion` major.

## 1. The state snapshot (the core payload)

This one JSON object is what a consumer receives from the point-in-time query AND as the payload of every stream event.
It is identical across all transports.

```json
{
  "schemaVersion": 1,
  "app": "scribe",
  "appVersion": "0.7.0",
  "status": "Recording",
  "dictating": true,
  "busy": true,
  "since": "2026-07-08T12:34:56.789Z",
  "updatedAt": "2026-07-08T12:34:56.812Z",
  "pid": 1234
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schemaVersion` | integer | Snapshot contract version. `1` for this document. |
| `app` | string | Constant `"scribe"`. Lets a generic consumer confirm which producer it reached. |
| `appVersion` | string | Scribe semver, for diagnostics and consumer feature-gating. |
| `status` | string | Raw internal status, PascalCase. One of the eight variants in section 2. The escape hatch for consumers that want their own logic. |
| `dictating` | boolean | Narrow flag: the microphone is actively capturing the user's voice. |
| `busy` | boolean | Broad flag: the user is inside a dictation cycle and external output would collide. Superset of `dictating`. |
| `since` | string (ISO-8601 UTC, millisecond precision, `Z`) | When the current `status` was entered. |
| `updatedAt` | string (ISO-8601 UTC, millisecond precision, `Z`) | When this snapshot was produced. Refreshed on a heartbeat, not only on transitions. |
| `pid` | integer | Scribe's OS process id, for file-fallback liveness checks. |

Hard invariant: `dictating == true` implies `busy == true`.

The `status` enum MAY gain variants in a future version.
Consumers MUST NOT assume the set is closed.
Prefer the two booleans (which Scribe always computes correctly, even for a new status) over switching on the raw `status` string, unless you deliberately opt into that coupling.

## 2. The dictation model - mapping `status` to the two booleans

Scribe owns this mapping so no consumer re-implements "what counts as dictating," and the meaning cannot drift between consumers.
The eight `status` values and their booleans:

| `status` | `dictating` | `busy` | Meaning |
| --- | --- | --- | --- |
| `Idle` | false | false | Nothing happening. |
| `Recording` | true | true | Mic open, user is speaking. |
| `Stopping` | true | true | Stop requested, audio tail still flushing; the user may still be finishing a word. |
| `Transcribing` | false | true | Mic closed, Whisper is turning the audio into text; output now would land on top of the result. |
| `Pasting` | false | true | Scribe is inserting text into the focused app; output now would interleave with the paste. |
| `Ready` | false | false | Transient post-paste state that self-heals to `Idle`. The cycle is done. |
| `Error` | false | false | Transcription failed; auto-heals in ~5s. Nothing is being captured. |
| `Paused` | false | false | Hotkey disabled; the user has explicitly turned dictation off. |

Consumer guidance:

- A "hold my output so I don't talk over the user" consumer (TTS, another agent, a notifier) gates on `busy`.
- A "show the mic state" consumer (a status pill, an indicator) gates on `dictating`.
- `dictating` is exactly the legacy `listening` field (see section 6); it is a faithful, backward-compatible rename.

## 3. Transport: localhost HTTP + SSE (canonical)

The canonical base transport is an in-process HTTP + Server-Sent-Events server the Scribe app runs on a loopback port for its whole lifetime.

- Bind: `127.0.0.1` only (loopback), never `0.0.0.0`.
- Port: ephemeral, chosen at startup. Discover it from the discovery file (section 5); never hard-code it.
- HTTP/1.1.

### 3.1 `GET /v1/status`

Returns the current state snapshot (section 1) as a point-in-time query.

- `200 OK`, `Content-Type: application/json`, body is the snapshot object.
- `401 Unauthorized` if the read token is missing or wrong (section 4).
- `404 Not Found` for any other path.

### 3.2 `GET /v1/events`

An SSE stream (`Content-Type: text/event-stream`) of state changes.

- `200 OK`, then an open stream that stays open until Scribe exits or the consumer disconnects.
- `401 Unauthorized` if the read token is missing or wrong.

Event frames use standard SSE framing. Three named events, each carrying the full snapshot (section 1) as the `data:` line:

| SSE `event:` | When | `data:` |
| --- | --- | --- |
| `state` | On connect (replay), then on EVERY state change. The firehose and source of truth. | full snapshot |
| `dictation.started` | The instant `dictating` flips false -> true (entering `Recording`). | full snapshot |
| `dictation.stopped` | The instant `dictating` flips true -> false (leaving `Stopping`). | full snapshot |

Example frame:

```
event: state
data: {"schemaVersion":1,"app":"scribe","appVersion":"0.7.0","status":"Recording","dictating":true,"busy":true,"since":"2026-07-08T12:34:56.789Z","updatedAt":"2026-07-08T12:34:56.812Z","pid":1234}

```

Guarantees:

- On connect, the server IMMEDIATELY sends the current snapshot as a `state` event, before any live change.
  This closes the race between "query now" and "subscribe": a consumer only needs to open the stream, never a separate GET.
- Each state change produces exactly one `state` event.
- A `dictation.started` / `dictation.stopped` fires ONLY on the matching edge of the `dictating` boolean, never spuriously.
  A transition that does not flip `dictating` (e.g. `Transcribing` -> `Ready`) emits only `state`.
- Ordering: events on a single connection are totally ordered.
  Within one transition, the `state` event is emitted first, then the edge event if any; both carry the same snapshot.
- Consumers MUST be level-triggered on the snapshot.
  Act on the current `busy` / `dictating` from the latest `state` (or the initial replay); use the edge events only to react faster.
  This makes reconnects safe: the replayed snapshot always re-establishes truth.
- Heartbeat: the server emits an SSE comment line `: ping` roughly every 10 seconds so a consumer can detect a hung/half-open connection.
  Recommended consumer read timeout: ~30 seconds. Treat prolonged silence as death: revert to not-dictating and reconnect.

## 4. Auth

Loopback bind is not isolation on a shared machine (any local process can reach `127.0.0.1`), so every request carries a per-process read token.

- The token is a random string minted fresh each time Scribe starts.
  It is published in the discovery file (section 5); only a process that can read the user's home directory can obtain it.
- Primary: send it as an HTTP header - `Authorization: Bearer <readToken>`.
- Fallback: send it as a query parameter - `?token=<readToken>`.
  This exists because a browser `EventSource` cannot set request headers. Both `/v1/status` and `/v1/events` accept either form; the header takes precedence when both are present.
- A missing or wrong token yields `401 Unauthorized`.
- The v1 surface is READ-ONLY. There are no mutation endpoints, so `readToken` is the only credential and a consumer can never change Scribe's state.

## 5. Discovery - `~/.scribe/control.json`

A consumer finds the running server by reading a well-known file in the user's home directory.

- Canonical path: `~/.scribe/control.json`
  (on Windows, `%USERPROFILE%\.scribe\control.json`).
- The Scribe Dev flavor (which can run alongside stable Scribe) writes `~/.scribe/control.dev.json` instead, so the two never clobber each other.
  A consumer targeting normal Scribe reads `control.json`.
- Written atomically (temp file + rename) once the server has bound its port.
  Overwritten on startup; removed on graceful shutdown.
- Best-effort restrictive file permissions.

```json
{
  "schemaVersion": 1,
  "app": "scribe",
  "appVersion": "0.7.0",
  "pid": 1234,
  "transport": "http-sse",
  "baseUrl": "http://127.0.0.1:52431",
  "endpoints": { "status": "/v1/status", "events": "/v1/events" },
  "readToken": "00000000-0000-0000-0000-000000000000",
  "updatedAt": "2026-07-08T12:34:56.000Z"
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schemaVersion` | integer | Discovery-file contract version. `1` for this document. |
| `app` | string | Constant `"scribe"`. |
| `appVersion` | string | Scribe semver. |
| `pid` | integer | Scribe's process id. Lets a consumer pid-check discovery itself. |
| `transport` | string | Constant `"http-sse"` for v1. Names the transport the URLs below speak. |
| `baseUrl` | string | Loopback origin, including the ephemeral port. Prepend to an endpoint path. |
| `endpoints` | object | Map of logical endpoint name to path. `status` and `events` for v1. Use these rather than hard-coding paths. |
| `readToken` | string | The per-process read token (section 4). |
| `updatedAt` | string (ISO-8601 UTC) | When this file was written. |

Consumer flow:

1. Read `~/.scribe/control.json`. Missing or unparseable -> Scribe not running -> not dictating.
2. Optionally pid-check `pid`; if dead, treat the file as void (Scribe crashed without cleaning up).
3. Build a request URL as `baseUrl + endpoints.status` (or `endpoints.events`), attach the token, and connect.

## 6. File fallback - `status.json`

For zero-dependency consumers that cannot or will not open a socket, Scribe also mirrors the snapshot to an on-disk file.
This is a fallback; the HTTP transport is canonical.

- Location: the Scribe app cache directory, filename `status.json`.
  (This is the Tauri per-app cache dir, which already gives dev/stable separation. It is NOT in `~/.scribe/`.)
- Written atomically (temp file + rename) next to every state change, AND re-written on a periodic heartbeat every ~5 seconds so `updatedAt` stays fresh during a long recording.
- Graceful shutdown writes a final `Idle` snapshot.

Shape (a superset of the section-1 snapshot):

```json
{
  "schemaVersion": 1,
  "app": "scribe",
  "appVersion": "0.7.0",
  "status": "Recording",
  "dictating": true,
  "busy": true,
  "listening": true,
  "since": "2026-07-08T12:34:56.789Z",
  "updatedAt": "2026-07-08T12:34:56.812Z",
  "pid": 1234
}
```

- All section-1 fields carry the same meaning.
- `listening` is a DEPRECATED alias of `dictating`, kept for one release for existing readers. New consumers MUST read `dictating`. It will be removed in a future version.

## 7. Liveness and staleness (the hard requirement)

A crashed or dead Scribe must NEVER leave a consumer believing dictation is still active.
Stale-or-dead ALWAYS resolves to not-dictating. This is satisfied per transport:

### 7.1 HTTP + SSE (intrinsic)

- If Scribe is not running, connecting to the loopback port is refused (`ECONNREFUSED`).
  A consumer treats "cannot connect" as "not running, therefore not dictating."
- If Scribe crashes mid-stream, the TCP connection drops and the SSE stream closes (EOF).
  A consumer treats stream-closed as "revert to not dictating" immediately, then reconnects with backoff; the initial `state` replay re-establishes truth on reconnect.
- The `: ping` heartbeat (~10s) lets a consumer detect a hung/half-open connection via a read timeout (~30s recommended) and treat silence as death.
- A crashed producer therefore cannot wedge a consumer into a permanent "dictating" belief: the belief dies with the socket.

### 7.2 File fallback (pid + TTL)

A consumer reading `status.json` MUST apply this exact algorithm:

1. File missing or unparseable -> not dictating (Scribe not running, or still starting).
2. `pid` not alive (authoritative kill-switch; `OpenProcess` on Windows, `kill -0` on Unix) -> not dictating, regardless of the file's contents.
3. `now - updatedAt > TTL` -> unknown -> treat as not dictating.
4. Otherwise -> trust `dictating` / `busy` from the file.

- Heartbeat interval: ~5 seconds (Scribe re-writes `status.json` this often while alive).
- Recommended TTL: 15 seconds (3x the heartbeat).
- "Unknown -> not dictating" deliberately fails toward "you may act," the only safe failure for the hard requirement.
- Graceful shutdown already writes a final `Idle` snapshot, so an ordered exit leaves the file correct without waiting out the TTL.

## 8. Minimal consumer recipes

Point-in-time query (shell):

```sh
CTL="$HOME/.scribe/control.json"
BASE=$(jq -r .baseUrl "$CTL"); TOKEN=$(jq -r .readToken "$CTL")
curl -s -H "Authorization: Bearer $TOKEN" "$BASE/v1/status" | jq '{dictating, busy, status}'
```

Stream (shell):

```sh
curl -sN -H "Authorization: Bearer $TOKEN" "$BASE/v1/events"
```

Browser (`EventSource`, header not settable, so query token):

```js
const es = new EventSource(`${baseUrl}/v1/events?token=${readToken}`);
es.addEventListener("state", (e) => {
  const s = JSON.parse(e.data);
  // level-triggered: act on s.busy / s.dictating every time
});
```
