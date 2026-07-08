# scribe-cli

A thin, standalone command-line client for **Scribe's dictation-state interface**.

Scribe (a Windows dictation app) runs an in-process localhost HTTP+SSE server and publishes a discovery file so any tool can ask *"is the user dictating right now?"* and stream start/stop.
`scribe-cli` is a general client of that interface: a human or a shell script can query the state once or follow it live.

It speaks only the frozen **v1 wire contract** (HTTP + Server-Sent-Events over loopback).
It is decoupled from Scribe's Tauri GUI, depends on none of it, and builds and ships in isolation on any platform.
It makes no assumptions about who is consuming it.

## Install / build

```sh
cargo build --release        # produces target/release/scribe
```

The binary is named `scribe`.

## Commands

### `scribe status` - point-in-time query

Reads `~/.scribe/control.json`, resolves `baseUrl` + `readToken`, GETs `/v1/status` with `Authorization: Bearer <readToken>`, and prints the snapshot.

```sh
scribe status                 # human-readable summary
scribe status --json          # raw snapshot JSON (one line), ideal for `jq`
scribe status --quiet         # print nothing; exit code gates on `busy`
scribe status --quiet --field dictating   # gate on `dictating` instead
```

Human output:

```
Scribe: dictating (Recording)
  dictating:  true
  busy:       true
  status:     Recording
  since:      2026-07-08T12:34:56.789Z
  updatedAt:  2026-07-08T12:34:56.812Z
  app:        scribe 0.7.0  pid 1234
```

`--json` emits the **raw** server payload verbatim, so additive/unknown fields (and future ones) are preserved with full fidelity.

#### Exit codes (`status`)

Default (non-`--quiet`) mode - meant for "did I reach Scribe?":

| Code | Meaning |
| --- | --- |
| `0` | Reached Scribe; a live snapshot was printed. |
| `1` | Reached Scribe but errored (bad read token / protocol failure). Still prints a not-dictating snapshot. |
| `2` | Usage error (bad flags). |
| `3` | Offline - Scribe not running / unreachable. Prints a not-dictating snapshot; the reason goes to stderr. |

`--quiet` mode - meant for cheap gating in scripts. No output; the exit code alone reflects the gate field (`busy` by default, or `dictating` with `--field`):

| Code | Meaning |
| --- | --- |
| `0` | The gate field is **true** (e.g. Scribe is busy - hold your output). |
| `1` | The gate field is **false**, or Scribe is offline/unknown (safe to act). |

```sh
# Hold a text-to-speech announcement while the user is mid-dictation:
if scribe status --quiet; then echo "busy - waiting"; else say "your build is done"; fi
```

Offline/unknown deliberately gates to "safe to act" (exit 1), the only safe failure for the hard liveness requirement.

### `scribe watch` - stream state changes

Opens `/v1/events` (SSE) and streams. On connect the server replays the current snapshot, then emits one event per state change.

```sh
scribe watch                  # newline-delimited JSON, one snapshot per event
scribe watch --human          # concise human lines
scribe watch --no-reconnect   # exit after the first stream end instead of reconnecting
```

Default output is newline-delimited JSON (the raw snapshot per event), ideal for piping:

```sh
scribe watch | jq -c '{dictating, busy, status}'
```

**Level-triggered and self-healing.** Act on `busy`/`dictating` from each snapshot; the replay on (re)connect always re-establishes truth.
On stream close or a refused connection, `watch` emits a synthetic **not-dictating** snapshot (marked with `"offline": true`) and then reconnects with exponential backoff.
A crashed Scribe therefore can never leave a follower believing dictation is still active.

Stop it with Ctrl-C.

## Global options

All of these work with either subcommand, and each has an environment fallback:

| Flag | Env | Meaning |
| --- | --- | --- |
| `--dev` | `SCRIBE_CHANNEL=dev` | Target the Dev flavor (`~/.scribe/control.dev.json`) instead of `control.json`. |
| `--control <PATH>` | `SCRIBE_CONTROL` | Explicit control-file path, overriding discovery. |
| `--base-url <URL>` | `SCRIBE_BASE_URL` | Talk to a base URL directly, bypassing the control file. |
| `--token <TOKEN>` | `SCRIBE_TOKEN` | Read token for `Authorization: Bearer` (only needed with `--base-url`; discovery supplies it otherwise). |
| `--no-pid-check` | - | Skip the pid liveness check on the discovered process. |

## Liveness (the hard requirement)

Stale-or-dead **always** resolves to not-dictating (contract section 7). This client implements it exactly:

- `~/.scribe/control.json` missing or unparseable -> Scribe not running -> not-dictating (never hangs).
- The discovery file's `pid` is checked for liveness (`kill -0` on Unix, `OpenProcess` on Windows); a dead process voids the file. Disable with `--no-pid-check`.
- Connection refused / stream closed -> not-dictating, then reconnect with backoff (`watch`) or report offline with exit 3 (`status`).
- A discovery or snapshot `schemaVersion` newer than v1 is rejected (discovery) or warned about while trusting only the two booleans (snapshot).

## The snapshot

Every query and every event carries the same JSON object. The fields this client relies on:

| Field | Meaning |
| --- | --- |
| `dictating` | Mic is actively capturing the user's voice. |
| `busy` | User is inside a dictation cycle; external output would collide. Superset of `dictating`. |
| `status` | Raw internal status (PascalCase). The set is **not** closed - prefer the booleans. |
| `since` / `updatedAt` | ISO-8601 UTC, millisecond precision. |
| `pid` | Scribe's process id. |

Gate a "don't talk over the user" tool on `busy`; gate a "show the mic state" indicator on `dictating`.

## Design notes

- **Standalone crate.** Not a member of the Scribe app's build; it pulls in none of the Tauri/GUI dependencies. It reuses the same HTTP stack the app already uses (blocking `reqwest` over rustls) but needs no async runtime.
- **Raw-fidelity JSON.** `status --json` and `watch` emit the exact bytes the server sent, so nothing is lost to re-serialization.
- **Testable core.** Discovery parsing, the liveness/staleness decision, snapshot decode, and the SSE parser are all unit-tested; behavior (status query, watch + reconnect, 401, offline) is tested against an in-process mock HTTP+SSE server.
