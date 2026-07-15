# Scribe Dictation MCP Server

A standalone [Model Context Protocol](https://modelcontextprotocol.io) server that exposes **Scribe's live dictation state** to any MCP host (Claude, other AI agents, MCP-speaking tools).

It is a thin, generic **client** of Scribe's frozen HTTP+SSE wire contract (v1) - see the authoritative [`docs/integrations/dictation-state-contract.md`](../../docs/integrations/dictation-state-contract.md), owned by the Scribe producer.
It does not embed, link, or depend on the Scribe desktop app; it discovers a running Scribe at runtime and proxies its state over MCP.
Nothing here is specific to any particular host.

## What it exposes

| Kind | Name | Purpose |
| --- | --- | --- |
| Tool | `get_dictation_state` | Point-in-time snapshot: `{ online, dictating, busy, status, since, updatedAt, pid, appVersion, ... }`. |
| Tool | `is_dictating` | Cheap gate: `{ dictating, busy, online, status }`. |
| Resource | `scribe://dictation/state` | Live dictation state as JSON. **Subscribable** - the host is notified the instant dictation starts or stops. |

Transport: **stdio** (the host launches this process and speaks MCP on stdin/stdout).

### The state view

Every tool/resource returns a normalized view. The two booleans are authoritative:

- `dictating` - the mic is actively capturing (contract's narrow flag).
- `busy` - the user is inside a dictation cycle; external output would collide (broad flag, superset of `dictating`).

Gate "hold my output so I don't talk over the user" on **`busy`**; gate a mic indicator on **`dictating`**.

`online` reports whether a live Scribe was reachable. **When Scribe is offline, dead, or unreachable, `dictating` and `busy` are always `false`** and `status` is `"Offline"` with a `reason`. A crashed Scribe can never wedge a consumer into a stuck "dictating" belief (contract sections 5 + 7).

## How it works

1. Reads `~/.scribe/control.json` to discover Scribe's `baseUrl`, `readToken`, and endpoints. Missing / unparseable / schema-too-new / dead-`pid` all resolve to **not-dictating**.
2. `get_dictation_state` / `is_dictating` do a fresh authenticated `GET /v1/status` per call (a true point-in-time query).
3. The resource is driven by a background SSE subscription to `GET /v1/events`. `state` / `dictation.started` / `dictation.stopped` events update the resource and fire `notifications/resources/updated` to subscribers.
4. The subscription is **level-triggered** on the snapshot and self-healing: a dropped stream, refused connection, or ~30s of silence reverts to not-dictating immediately, then reconnects with backoff. On reconnect, the contract's initial `state` replay re-establishes truth. If Scribe restarts on a new port, the control file is re-read on each reconnect.

## Install & build

```sh
cd integrations/mcp-server
npm install
npm run build   # emits dist/
```

## Configure a host to launch it

Point your MCP host at the built entry with Node. Example (`claude_desktop_config.json` / any host's MCP server config):

```json
{
  "mcpServers": {
    "scribe-dictation": {
      "command": "node",
      "args": ["/absolute/path/to/scribe/integrations/mcp-server/dist/index.js"]
    }
  }
}
```

For Claude Code:

```sh
claude mcp add scribe-dictation -- node /absolute/path/to/scribe/integrations/mcp-server/dist/index.js
```

### Options (flags + env)

| Flag | Env | Default | Meaning |
| --- | --- | --- | --- |
| `--dev` | `SCRIBE_MCP_DEV=1` | off | Target the Scribe **Dev flavor** (`~/.scribe/control.dev.json`). |
| `--control <path>` | `SCRIBE_CONTROL_FILE` | `~/.scribe/control.json` | Explicit control-file path (wins over `--dev`). |
| - | `SCRIBE_MCP_HTTP_TIMEOUT_MS` | `5000` | Point-in-time HTTP timeout. |
| - | `SCRIBE_MCP_IDLE_TIMEOUT_MS` | `30000` | SSE idle read-timeout (silence beyond this = death). |
| - | `SCRIBE_MCP_MIN_BACKOFF_MS` | `500` | Reconnect backoff floor. |
| - | `SCRIBE_MCP_MAX_BACKOFF_MS` | `10000` | Reconnect backoff ceiling. |
| - | `SCRIBE_MCP_DEBUG=1` | off | Emit diagnostics to **stderr** (stdout is reserved for the protocol). |

## Develop, test, smoke

```sh
npm run typecheck   # tsc --noEmit
npm test            # unit + integration tests against a mock HTTP+SSE server
npm run smoke       # build, then drive the real built server over stdio end-to-end
```

Tests run against an in-repo mock of Scribe's `/v1/status` + `/v1/events` (`test/mock-server.ts`), covering discovery parsing, the SSE-to-notification mapping, the offline/not-dictating fallbacks, and the "never stuck dictating" liveness guarantee - Scribe itself is Windows-only and is not required to run the suite.

You can also poke it manually with the MCP Inspector:

```sh
npx @modelcontextprotocol/inspector node dist/index.js
```

## Versioning

This package is versioned independently, starting at `0.1.0`. It does **not** track the Scribe app version. It targets snapshot/discovery `schemaVersion: 1` and rejects any payload whose major version is newer than it understands.
