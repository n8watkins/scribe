# Dictation State Interface - Design Proposal

Status: proposal (design phase, no production code).
Author: design crew (branch `scribe-state-interface-design`).
Scribe version at time of writing: 0.6.1.

## 1. Purpose and framing

Scribe should offer a first-class, documented integration surface that ANY external tool can connect to in order to read Scribe's live dictation state.
Two consumer needs must be served:

1. A point-in-time query - "is the user dictating RIGHT NOW?"
2. A stream of start / stop events - so a consumer can hold its own output the instant dictation begins and resume the instant it ends.

The general's underlying need is "do not talk over me."
That window is broader than just the microphone being open, so the contract must let each consumer choose how conservative it wants to be.

This is Scribe OFFERING an API, not a bespoke "T-Hub to Scribe" bridge.
The contract carries no T-Hub-specific assumptions.
T-Hub is merely the first consumer; a shell script, a text-to-speech daemon, a notification manager, or another agent must be able to consume it just as easily.

Today the only seam is an undocumented status file (`status.json` under the app cache dir) that consumers read directly (`app/src-tauri/src/status_file.rs`).
This proposal replaces that brittle seam with a documented, connection-based interface and keeps a hardened file as an explicit fallback.

## 2. What the code actually does today (verified)

All citations are against this worktree.

### 2.1 Single source of truth

- The one source of truth is the `AppStateMachine` in `app/src-tauri/src/app_state.rs:73-88`, stored in `BackendState::app_state: Mutex<AppStateMachine>` (`app/src-tauri/src/commands.rs:23`).
- `AppStatus` (`app/src-tauri/src/app_state.rs:4-14`) has exactly eight variants: `Idle`, `Recording`, `Stopping`, `Transcribing`, `Pasting`, `Ready`, `Error`, `Paused`.
- Legal transitions are enforced by `AppStateMachine::transition()` (`app/src-tauri/src/app_state.rs:103-146`).
- The public snapshot type is `AppStateSnapshot` (`app/src-tauri/src/app_state.rs:65-71`): `status`, `error: Option<AppErrorInfo>`, `updatedAt` (camelCase over the wire).

### 2.2 The state-change choke point

Every state change flows through a small `emit_state_snapshot` helper that is duplicated in three modules and does exactly two things:

```rust
// app/src-tauri/src/audio.rs:788-792 (also dictation.rs:633-637, output.rs:485-489)
fn emit_state_snapshot(app: &AppHandle, snapshot: &AppStateSnapshot) {
    let _ = app.emit("scribe:app-state", snapshot);   // Tauri event to the webview
    crate::status_file::publish(app, snapshot);        // mirror to status.json on disk
}
```

This helper is the natural and only place to add a third line that broadcasts to a new server.
Adding the interface does not require touching any transition logic.

### 2.3 Events emitted today

- `scribe:app-state` - full `AppStateSnapshot`, emitted on every transition (`audio.rs:789`, `dictation.rs:634`, `output.rs:486`).
- `audio://recording-started` - `RecordingSessionInfo` (`app/src-tauri/src/audio.rs:393`).
- `audio://recording-stopped` - `RecordingResult` (`audio.rs:772`, plus cancel/test paths at 562/613).
- `audio://recording-timeout`, `audio://level` - recording internals, not state.

These are Tauri webview events, not an external surface.

### 2.4 The status file today

`app/src-tauri/src/status_file.rs` writes `status.json` into `app.path().app_cache_dir()` (`status_file.rs:82-99`), atomically via a pid-tagged temp file plus rename (`status_file.rs:58-76`).
Shape: `status` (PascalCase enum string), `listening` (bool), `since`, `updatedAt`, `pid` (`status_file.rs:21-38`).
`listening` is true only for `Recording` or `Stopping` (`status_file.rs:40-44`).
It is written next to every transition via `publish()`, and `publish_idle()` runs at startup (`lib.rs:385`) and shutdown (`lib.rs:488`).
Key limitation: it is only rewritten on transitions, so during a long recording `updatedAt` goes stale even though Scribe is alive and dictating.

### 2.5 Correction to prior recon - there is NO in-process server today

The prior recon stated that `whisper_server.rs` "binds 127.0.0.1 on an ephemeral port to keep the whisper model warm," implying an in-process HTTP server precedent.
That is not what the code does.
`whisper_server.rs:610-618` binds a `TcpListener` only to discover a free port, immediately drops the listener, and then spawns an EXTERNAL `whisper-server.exe` process; Scribe is only an HTTP client to it via a blocking `reqwest` client (`app/src-tauri/Cargo.toml`, `reqwest = { version = "0.12", features = ["blocking", ...] }`).
There is no `tokio` runtime and no server crate (`axum`, `hyper`, `warp`, `tiny_http`) in the dependency tree; the app's concurrency model is `std::thread` throughout (for example the whisper watchdog at `whisper_server.rs:380-407`).

Consequences for this design:

- Standing up an in-process server is a genuine new capability, not a copy of an existing one, and this proposal treats it as such.
- The free-port-via-`TcpListener` idiom (`whisper_server.rs:610-618`) and the spawn-a-thread-in-setup idiom are reusable; the HTTP serving itself is new.
- To stay faithful to the existing `std::thread`/blocking model and avoid pulling in an async runtime, the base transport should prefer a small synchronous HTTP server (for example `tiny_http`) on a dedicated thread, with hand-written Server-Sent Events framing (SSE is trivial to emit as `text/event-stream`).
  An `axum` + `tokio` server is the more idiomatic SSE stack but introduces an async runtime the codebase does not otherwise use; the trade is called out again in the build breakdown.

## 3. Part A - The contract (transport-independent)

The contract below is defined once and is identical no matter which transport (HTTP, CLI, MCP, or file) carries it.
Deriving all consumer-facing semantics inside Scribe, the process that owns the state machine, is a deliberate choice: consumers never re-implement "what counts as dictating," and the meaning cannot drift between consumers.

### 3.1 State snapshot schema

The object a consumer receives from a point-in-time query, and the payload of every state event.

```json
{
  "schemaVersion": 1,
  "app": "scribe",
  "appVersion": "0.6.1",
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
| `schemaVersion` | integer | Contract version. Starts at 1. Bumped only on a breaking change. Consumers should accept unknown future minor additions and reject unknown majors. |
| `app` | string | Constant `"scribe"`. Lets a generic consumer confirm which producer it reached. |
| `appVersion` | string | Scribe semver, for diagnostics and consumer feature-gating. |
| `status` | string | Raw `AppStatus`, PascalCase, one of the eight variants in section 2.1. The escape hatch: consumers that want their own logic read this. |
| `dictating` | boolean | Narrow: the microphone is actively capturing the user's voice. |
| `busy` | boolean | Broad: the user is inside a dictation cycle and output would collide. Superset of `dictating`. |
| `since` | string (ISO-8601 UTC) | When the current `status` was entered (the state machine's `updatedAt`). |
| `updatedAt` | string (ISO-8601 UTC) | When this snapshot was produced. Refreshed on a heartbeat, not only on transitions (see 3.3). |
| `pid` | integer | Scribe's process id, for file-fallback liveness checks. |

Invariant: `dictating == true` implies `busy == true`.
The `status` enum may gain variants in future versions; consumers must not assume the set is closed, and should rely on the two booleans (which Scribe always computes correctly, even for new statuses) rather than switching on the raw string unless they opt into that coupling.

### 3.2 The dictation model - mapping `AppStatus` to booleans

The core design decision.
Two booleans, because "is the mic on" and "should I hold my output" are genuinely different questions with different answers, and different consumers have different tolerances.

| `AppStatus` | `dictating` | `busy` | Rationale |
| --- | --- | --- | --- |
| `Idle` | false | false | Nothing happening. |
| `Recording` | true | true | Mic open, user is speaking. |
| `Stopping` | true | true | Stop requested, audio tail still flushing; the user may still be finishing a word. Momentary. |
| `Transcribing` | false | true | Mic closed, but Whisper is turning the just-spoken audio into text. Output now would land on top of the result the user is waiting for. |
| `Pasting` | false | true | Scribe is inserting text into the focused app. A consumer stealing focus or emitting here would interleave with the paste. |
| `Ready` | false | false | Transient post-paste state that self-heals to `Idle` (`ReadyTimeout`). The cycle is done; the user may act. |
| `Error` | false | false | Transcription failed; auto-heals in ~5s (`dictation.rs:618`). Nothing is being captured. |
| `Paused` | false | false | Hotkey disabled; the user has explicitly turned dictation off. |

Why two flags rather than one:

- `dictating` answers the literal point-in-time query "is the user dictating right now" and is exactly today's `listening` (`Recording | Stopping`), so it is a faithful, backward-compatible rename.
- `busy` answers the general's real need, "do not talk over me," across the whole capture-to-insert cycle (`Recording | Stopping | Transcribing | Pasting`).
  A text-to-speech agent or another voice-producing tool must hold through `busy`; speaking during `Transcribing` or `Pasting` still talks over the moment.
  A purely visual tool (for example a status pill in another app) may only care about `dictating`.
- Exposing both, plus the raw `status`, means Scribe picks sensible defaults without locking any consumer into them.

Recommended default for a "hold your output" consumer: gate on `busy`.
Recommended default for a "show mic state" consumer: gate on `dictating`.

### 3.3 Events

Transport-neutral event names (mapped to each transport in Part B):

- `state` - the canonical event. Carries the full snapshot (section 3.1). Emitted on every state change. This is the firehose and the source of truth.
- `dictation.started` - emitted the instant `dictating` goes false to true (entering `Recording`). Carries the full snapshot.
- `dictation.stopped` - emitted the instant `dictating` goes true to false (leaving `Stopping`). Carries the full snapshot.

Notes and guarantees:

- On connect, the server immediately sends the current snapshot as a `state` event.
  This closes the race between "query now" and "subscribe": a consumer only needs to open the stream, never a separate GET.
- Each transition produces exactly one `state` event.
  A `dictation.started` / `dictation.stopped` fires only on the matching edge of the `dictating` boolean, never spuriously; a transition that does not flip `dictating` (for example `Transcribing` to `Ready`) emits only `state`.
- Ordering: events on a single connection are totally ordered (an SSE stream is ordered).
  Within one transition, the `state` event is emitted first, then the edge event if any; both carry the same snapshot.
- Consumers must be level-triggered on the snapshot and treat edge events as a low-latency nudge, not as the source of truth.
  Concretely: act on the current `busy` / `dictating` value from the latest `state` (or the initial replay), and use `dictation.started` / `dictation.stopped` only to react faster.
  This makes reconnects safe: there is no guarantee a consumer saw the `started` that matched a later `stopped`, but the replayed snapshot always re-establishes truth.
- A broader `busy` edge pair (`busy.started` / `busy.stopped`) may be added in a later schema version if a consumer needs edge latency on the broad flag; for v1 the `state` event plus a `busy` field is sufficient, since a consumer can watch `busy` on each `state`.

### 3.4 Liveness and staleness - handled Scribe-side (hard requirement)

A crashed Scribe must NEVER leave a consumer believing dictation is still active.
This is satisfied differently per transport, and the connection-based transport satisfies it intrinsically.

Connection-based transport (HTTP + SSE, the recommendation):

- Liveness is intrinsic. If Scribe is not running, a connection to the loopback port is refused (`ECONNREFUSED`).
  A consumer treats "cannot connect" as "Scribe not running, therefore not dictating."
- If Scribe crashes mid-stream, the TCP connection drops and the SSE stream closes (EOF).
  A consumer treats stream-closed as "revert to not dictating" immediately, then reconnects with backoff; the initial `state` replay re-establishes truth on reconnect.
  A crashed producer therefore cannot wedge a consumer into a permanent "dictating" belief - the belief dies with the socket.
- Heartbeat: the server emits an SSE comment ping (`: ping\n\n`) every ~10s so a consumer can detect a half-open or hung connection via a read timeout (recommended consumer read timeout ~30s) and treat silence as death.

File-based fallback (`status.json`, retained for zero-dependency consumers):

- pid-liveness (authoritative kill-switch): the consumer reads `pid` and checks whether that process is alive (on Windows, an `OpenProcess` / `kill -0` equivalent).
  If the pid is dead, the file is void; the consumer treats it as not-dictating, no matter what the file says.
- Staleness TTL (secondary): the consumer compares `now - updatedAt` against a TTL.
  To make the TTL meaningful, Scribe must re-write the file on a periodic heartbeat (recommended every 5s while the process is alive), not only on transitions, so `updatedAt` stays fresh during a long recording.
  Recommended TTL = 3x the heartbeat interval (~15s).
- Consumer interpretation algorithm (exact):
  1. File missing or unparseable -> not dictating (Scribe not running or still starting).
  2. `pid` not alive -> not dictating (crash backstop, authoritative).
  3. `now - updatedAt > TTL` -> unknown -> treat as not dictating.
  4. Otherwise -> trust `dictating` / `busy` from the file.
- The "unknown -> not dictating" choice deliberately fails toward "you may speak," which is the only safe failure for the hard requirement: a stale or dead producer can never trap a consumer in "hold forever."
  The heartbeat re-write is what prevents this branch from firing falsely while Scribe is genuinely dictating.
- Graceful shutdown already writes an `Idle` snapshot via `publish_idle()` (`lib.rs:488`), so an ordered exit leaves the file correct without waiting for the TTL.

### 3.5 Discovery and security

Discovery file (mirrors T-Hub's `~/.t-hub/control.json`, which was inspected and uses `addr` / `token` / `read_token` / `pid` / `protocol_version`):

- Canonical path: `~/.scribe/control.json` (on Windows `%USERPROFILE%\.scribe\control.json`).
  A home-dir well-known path is chosen over Tauri's bundle-identifier cache path because any external tool in any language can find it trivially and deterministically, exactly as tools already find T-Hub.
- Written atomically (temp + rename, reusing the `status_file.rs` idiom) once the server has bound its port; overwritten on startup and updated to reflect not-running on graceful shutdown.
- Shape:

```json
{
  "schemaVersion": 1,
  "app": "scribe",
  "appVersion": "0.6.1",
  "pid": 1234,
  "transport": "http-sse",
  "baseUrl": "http://127.0.0.1:52431",
  "endpoints": { "status": "/v1/status", "events": "/v1/events" },
  "readToken": "c3b54f25-7df6-4cfa-9ae2-342dd8d548c6",
  "updatedAt": "2026-07-08T12:34:56Z"
}
```

- `baseUrl` embeds the ephemeral loopback port chosen at bind time (free-port idiom from `whisper_server.rs:610-618`).
- `pid` in the discovery file lets a consumer pid-check discovery itself, the same way it pid-checks the status file.

Security (simple but not insecure):

- Bind loopback only (`127.0.0.1`), never `0.0.0.0`, matching the existing `SERVER_HOST` convention.
- Read token: loopback alone is not isolation on a shared machine, since any local process can reach `127.0.0.1`.
  A random `readToken` (generated fresh per process, like T-Hub's per-pid token) must be presented on every request.
  Because the token lives in `~/.scribe/control.json`, only a process that can read the user's home directory can obtain it.
- Token transport: native consumers send `Authorization: Bearer <readToken>`.
  Browser `EventSource` cannot set headers, so `/v1/events` also accepts `?token=<readToken>` as a fallback; the header path is primary.
- Read-only surface: v1 exposes no mutation endpoints, so the only credential is a `readToken`.
  A consumer can never change Scribe's state, which keeps the attack surface minimal.
- Best-effort restrictive file permissions on `control.json`.

## 4. Part B - Interface shape: comparison and recommendation

Weighed on quality, simplicity, robustness, scalability, and long-term maintainability.
Development cost is explicitly NOT weighed.

### 4.1 Option 1 - Localhost HTTP + SSE (in-process, always-on)

- `GET /v1/status` returns the snapshot (section 3.1).
- `GET /v1/events` is an SSE stream emitting `state`, `dictation.started`, `dictation.stopped` (section 3.3), starting with the current snapshot.
- Server bound at app setup on an ephemeral loopback port; discovery via `control.json`.

Strengths:

- Generic to the maximum degree: any language, any tool, `curl`-able and browser-testable. This is the "any external tool" requirement, met directly.
- Intrinsic liveness (section 3.4): the connection is the truth signal; a crash cannot wedge a consumer.
- Real-time push with trivial client code; SSE is a stock feature in every HTTP stack.
- Single source of truth: the server reads the live `AppStateMachine` and broadcasts from the existing `emit_state_snapshot` choke point.
- Scales to N consumers (N independent SSE connections) with no per-consumer process.
- Debuggable and self-describing; a human can inspect it with `curl`.

Weaknesses:

- Genuinely new capability: needs an HTTP server dependency, since none exists today (section 2.5).
  Mitigated by a small synchronous server (`tiny_http`) on a dedicated `std::thread`, which matches the existing concurrency model and avoids an async runtime.
- SSE header-auth limitation for browser `EventSource`; mitigated by the query-token fallback.

### 4.2 Option 2 - MCP server

- Dictation state as an MCP resource (`scribe://dictation-state`) plus a `get_dictation_state` tool, with MCP notifications for start/stop.

Strengths:

- First-class inside AI-agent ecosystems; typed schema; notifications map cleanly to the event contract.

Weaknesses:

- Not generic: only MCP-speaking consumers benefit. A shell script, a TTS daemon, or an arbitrary app cannot easily consume MCP. This fails the core "any tool" requirement as a base layer.
- Hosting model is awkward for an always-on GUI producer.
  MCP servers are usually spawned per client over stdio, which does not fit one long-lived app broadcasting to many consumers.
  A shared, long-lived MCP server needs the Streamable HTTP transport anyway, so you are back to hosting HTTP.
- "Who hosts the process" is unresolved: a per-consumer `scribe-mcp` child still needs a data source from the running Scribe, meaning it becomes a client of Option 1 or the file.
- Conclusion: MCP is excellent as a thin facade over the HTTP transport, and a poor choice as the base.

### 4.3 Option 3 - CLI

- `scribe status --json` prints the snapshot; `scribe watch` prints an NDJSON stream of events on stdout.

Strengths:

- Scriptable and unix-friendly; ideal for shell consumers, cron, and quick human checks.
- The consumer needs no port or token knowledge; the CLI handles discovery and auth internally.
- `scribe watch` exits when Scribe dies, giving natural liveness for a script.

Weaknesses:

- Not a data source: the CLI must still read from somewhere (the HTTP server, or the file). It cannot be the base layer either.
- Process-per-watch is heavier than a socket for high fan-out.
- Distribution overhead: Scribe is a GUI app, so a CLI binary must be shipped and put on PATH.
- Conclusion: excellent as a thin facade for scripts, not the base.

### 4.4 Layering - the key insight

The three options are not rivals; two of them are naturally facades over the first.

- Make the in-process HTTP + SSE server the single canonical base transport, reading the real `AppStateMachine` directly. One producer, one place where the dictation model is computed.
- Build the CLI as a thin client over HTTP: `scribe status` is a `GET /v1/status`, `scribe watch` streams `/v1/events` and reprints it as NDJSON. No duplicated state logic.
- Build the MCP server (if wanted) as a thin client over HTTP too: the resource, tool, and notifications are backed by the same GET and SSE.
- Keep `status.json` as a documented fallback, written by the same choke point, for zero-dependency or no-daemon consumers and for the pid + TTL liveness story.

This yields one authoritative producer with several optional, additive front-ends, and no divergent state logic - the best outcome on simplicity, robustness, scalability, and maintainability.

### 4.5 Recommendation

Primary recommendation: build the localhost HTTP + SSE server as the canonical base transport, with the layered CLI and MCP facades as optional later additions.

Phased path:

- Phase 1 (must): in-process HTTP + SSE server, `control.json` discovery, read token, the snapshot and event contract, and a hardened `status.json` fallback (new schema, heartbeat, documented pid + TTL).
  This alone fully satisfies T-Hub and any generic consumer.
- Phase 2 (ergonomics): the `scribe` CLI facade (`status`, `watch`) over HTTP.
- Phase 3 (agent-native, optional): the MCP facade over HTTP.

## 5. Part C - Build breakdown

Sized as crew-sized worktree tasks, assuming the general picks the HTTP + SSE base.
This is input for the captain's staffing.

### Task A - Contract and producer core (owns the version bump)

- Centralize the dictation model in one module: `status`, `dictating`, `busy`, and `schemaVersion`, with `is_dictating` (today's `is_listening`) and a new `is_busy`.
  Both the server and the status file consume this one module so the mapping cannot drift.
- Stand up the in-process HTTP + SSE server on an ephemeral loopback port at app setup (free-port idiom from `whisper_server.rs:610-618`, dedicated `std::thread`).
  Implement `GET /v1/status` and the `GET /v1/events` SSE stream with the initial-snapshot replay and heartbeat.
- Broadcast from the existing `emit_state_snapshot` choke point (`audio.rs:788`, `dictation.rs:633`, `output.rs:485`) into a channel that SSE connections subscribe to; no transition logic changes.
- Write `control.json` (discovery + `readToken`) atomically to `~/.scribe/`; refresh on startup and on graceful shutdown. Enforce the read-token check (header plus query fallback).
- Harden `status.json`: add `schemaVersion`, `dictating`, `busy`, `app`, `appVersion`; keep `status`, `since`, `updatedAt`, `pid`; keep `listening` as a deprecated alias of `dictating` for one release; add the periodic heartbeat re-write; document pid + TTL for consumers.
- Server-transport sub-decision to settle inside this task: `tiny_http` synchronous server on a thread (matches the `std::thread`/blocking model, no async runtime, hand-written SSE framing) versus `axum` + `tokio` (idiomatic SSE, new runtime).
  The design leans `tiny_http` for simplicity and fit; the task confirms with a spike.
- Owns the version bump (`app/package.json` + `app/src-tauri/Cargo.toml`).
  Per house rule, do not hand-edit `CHANGELOG.md`; leave it to the release tooling.
- Dependencies: none. Foundational. This is all of Phase 1.
- Optional finer grain: this task can be split into A1 (shared model module + `status.json` hardening) and A2 (server + discovery + auth) if the captain wants two smaller crews; A2 depends on A1.

### Task B - CLI facade

- `scribe status --json` and `scribe watch` implemented as thin clients over the HTTP transport, discovering `baseUrl` and `readToken` from `~/.scribe/control.json`.
- Ship and register the CLI binary.
- Dependencies: Task A. No state logic of its own.

### Task C - MCP facade (optional, later)

- MCP resource (`scribe://dictation-state`), a `get_dictation_state` tool, and start/stop notifications, all backed by the HTTP transport.
- Dependencies: Task A. No state logic of its own.

## 6. Open questions for the general

- Confirm the canonical discovery directory: `~/.scribe/` (recommended, mirrors T-Hub) versus the Tauri app data dir.
- Confirm whether `status.json` should physically move to `~/.scribe/` or be dual-written for one release for backward compatibility with current readers.
- Confirm the `tiny_http` versus `axum`/`tokio` lean before Task A starts, or delegate it to the Task A spike.
- Confirm that a read-only surface is sufficient for the foreseeable future (no consumer needs to command Scribe), so the token stays read-only.
