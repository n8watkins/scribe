/**
 * HTTP + SSE client for the frozen wire contract (sections 3 + 4).
 *
 * `fetchSnapshot` performs the point-in-time `GET /v1/status`. `streamEvents`
 * drives the SSE subscription, parsing standard SSE framing and enforcing an
 * idle read-timeout so a hung/half-open connection is treated as death.
 * Both authenticate with `Authorization: Bearer <readToken>`.
 */

import type { DiscoveryDoc } from "./discovery.js";
import { endpointUrl } from "./discovery.js";
import type { OfflineReason, ScribeSnapshot } from "./types.js";
import { parseSnapshot } from "./types.js";

export type FetchSnapshotResult =
  | { ok: true; snapshot: ScribeSnapshot }
  | { ok: false; reason: OfflineReason };

/** Map a thrown fetch/stream error to a not-dictating offline reason. */
export function reasonFromError(err: unknown): OfflineReason {
  const e = err as { name?: string; message?: string; cause?: { code?: string } };
  if (e?.name === "AbortError" || e?.message === "idle-timeout") return "stream-timeout";
  const code = e?.cause?.code;
  if (code === "ECONNREFUSED" || code === "ECONNRESET" || code === "ENOTFOUND") {
    return "connection-refused";
  }
  return "connection-refused";
}

/** Point-in-time `GET /v1/status`. Never throws; failures become reasons. */
export async function fetchSnapshot(
  doc: DiscoveryDoc,
  opts: { timeoutMs?: number } = {},
): Promise<FetchSnapshotResult> {
  const url = endpointUrl(doc, "status");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), opts.timeoutMs ?? 5000);
  try {
    const res = await fetch(url, {
      headers: { Authorization: `Bearer ${doc.readToken}`, Accept: "application/json" },
      signal: controller.signal,
    });
    if (!res.ok) return { ok: false, reason: "http-error" };
    const json = (await res.json()) as unknown;
    const snap = parseSnapshot(json);
    if (!snap) return { ok: false, reason: "http-error" };
    return { ok: true, snapshot: snap };
  } catch (err) {
    return { ok: false, reason: reasonFromError(err) };
  } finally {
    clearTimeout(timer);
  }
}

export interface StreamOptions {
  /** Fired once the stream is open (HTTP 200). */
  onOpen?: () => void;
  /** Fired for every SSE event carrying a valid snapshot. */
  onEvent: (event: string, snapshot: ScribeSnapshot) => void;
  /** Abort the read if no bytes (data OR heartbeat) arrive within this window. */
  idleTimeoutMs?: number;
  /** External abort - shutdown, or a supervising reconnect loop. */
  signal?: AbortSignal;
}

/** Split one raw SSE frame into its event name + snapshot, if any. */
function handleFrame(raw: string, onEvent: StreamOptions["onEvent"]): void {
  if (raw.trim().length === 0) return;
  let event = "message";
  const dataLines: string[] = [];
  for (let line of raw.split("\n")) {
    if (line.endsWith("\r")) line = line.slice(0, -1);
    if (line.length === 0 || line.startsWith(":")) continue; // blank or comment (heartbeat)
    const colon = line.indexOf(":");
    let field: string;
    let value: string;
    if (colon === -1) {
      field = line;
      value = "";
    } else {
      field = line.slice(0, colon);
      value = line.slice(colon + 1);
      if (value.startsWith(" ")) value = value.slice(1);
    }
    if (field === "event") event = value;
    else if (field === "data") dataLines.push(value);
  }
  if (dataLines.length === 0) return;
  let json: unknown;
  try {
    json = JSON.parse(dataLines.join("\n"));
  } catch {
    return;
  }
  const snap = parseSnapshot(json);
  if (snap) onEvent(event, snap);
}

/**
 * Open `GET /v1/events` and pump frames to `onEvent` until the stream ends, the
 * idle timeout fires, or the external signal aborts. Resolves on clean EOF;
 * throws on transport error or timeout (the caller reverts to not-dictating and
 * reconnects). The contract guarantees an immediate `state` replay on connect.
 */
export async function streamEvents(doc: DiscoveryDoc, opts: StreamOptions): Promise<void> {
  const url = endpointUrl(doc, "events");
  const controller = new AbortController();
  const idleMs = opts.idleTimeoutMs ?? 30_000;

  const onExternalAbort = () => controller.abort();
  opts.signal?.addEventListener("abort", onExternalAbort, { once: true });

  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  const resetIdle = () => {
    if (idleTimer) clearTimeout(idleTimer);
    idleTimer = setTimeout(() => controller.abort(new Error("idle-timeout")), idleMs);
  };

  try {
    const res = await fetch(url, {
      headers: { Authorization: `Bearer ${doc.readToken}`, Accept: "text/event-stream" },
      signal: controller.signal,
    });
    if (!res.ok || !res.body) {
      throw Object.assign(new Error(`events HTTP ${res.status}`), { httpStatus: res.status });
    }
    opts.onOpen?.();
    resetIdle();

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      resetIdle();
      buffer += decoder.decode(value, { stream: true });
      // Frames are separated by a blank line (\n\n or \r\n\r\n).
      for (;;) {
        const n = buffer.indexOf("\n\n");
        const r = buffer.indexOf("\r\n\r\n");
        let cut = -1;
        let width = 0;
        if (n !== -1 && (r === -1 || n < r)) {
          cut = n;
          width = 2;
        } else if (r !== -1) {
          cut = r;
          width = 4;
        }
        if (cut === -1) break;
        const frame = buffer.slice(0, cut);
        buffer = buffer.slice(cut + width);
        handleFrame(frame, opts.onEvent);
      }
    }
  } finally {
    if (idleTimer) clearTimeout(idleTimer);
    opts.signal?.removeEventListener("abort", onExternalAbort);
  }
}
