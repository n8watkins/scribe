/**
 * StateEngine - the single source of truth for the live dictation resource.
 *
 * It maintains a background SSE subscription with reconnect/backoff, maps
 * `state` / `dictation.started` / `dictation.stopped` events to a normalized
 * `DictationState`, and notifies listeners on every meaningful change. Any
 * disconnect, timeout, refusal, dead pid, or missing control file drives the
 * state to offline (= not-dictating) immediately, so the engine can never get
 * wedged reporting a stuck "dictating" (contract section 7).
 *
 * `refreshNow()` is an independent one-shot point-in-time query used by the
 * tools; it never disturbs the subscription.
 */

import { readDiscovery } from "./discovery.js";
import { fetchSnapshot, reasonFromError, streamEvents } from "./http.js";
import type { DictationState, OfflineReason } from "./types.js";
import { offlineState, onlineState } from "./types.js";

export interface EngineOptions {
  /** Absolute path to the control file to read on every (re)connect. */
  controlPath: string;
  /** Point-in-time HTTP timeout. Default 5000ms. */
  httpTimeoutMs?: number;
  /** SSE idle read-timeout - silence beyond this is treated as death. Default 30000ms. */
  idleTimeoutMs?: number;
  /** Reconnect backoff floor. Default 500ms. */
  minBackoffMs?: number;
  /** Reconnect backoff ceiling. Default 10000ms. */
  maxBackoffMs?: number;
  /** Optional stderr logger (never stdout - that carries the MCP protocol). */
  log?: (msg: string) => void;
}

type Listener = (state: DictationState) => void;

export class StateEngine {
  private current: DictationState = offlineState("not-connected-yet");
  private readonly listeners = new Set<Listener>();
  private controller: AbortController | undefined;
  private looping = false;

  private readonly controlPath: string;
  private readonly httpTimeoutMs: number;
  private readonly idleTimeoutMs: number;
  private readonly minBackoffMs: number;
  private readonly maxBackoffMs: number;
  private readonly log: (msg: string) => void;

  constructor(opts: EngineOptions) {
    this.controlPath = opts.controlPath;
    this.httpTimeoutMs = opts.httpTimeoutMs ?? 5000;
    this.idleTimeoutMs = opts.idleTimeoutMs ?? 30_000;
    this.minBackoffMs = opts.minBackoffMs ?? 500;
    this.maxBackoffMs = opts.maxBackoffMs ?? 10_000;
    this.log = opts.log ?? (() => {});
  }

  /** The latest known state (SSE-driven). Cheap, non-blocking. */
  getCurrent(): DictationState {
    return this.current;
  }

  /** Subscribe to change notifications. Returns an unsubscribe function. */
  onChange(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /**
   * One-shot authoritative point-in-time query: read discovery, GET /v1/status.
   * Does NOT mutate `current` or touch the subscription. Offline maps to
   * not-dictating.
   */
  async refreshNow(): Promise<DictationState> {
    const disc = await readDiscovery(this.controlPath);
    if (!disc.ok) return offlineState(disc.reason);
    const r = await fetchSnapshot(disc.doc, { timeoutMs: this.httpTimeoutMs });
    if (!r.ok) return offlineState(r.reason);
    return onlineState(r.snapshot);
  }

  /** Begin (or resume) the background subscription loop. Idempotent. */
  start(): void {
    if (this.controller) return;
    this.controller = new AbortController();
    void this.loop(this.controller.signal);
  }

  /** Stop the subscription loop and revert to not-dictating. */
  async stop(): Promise<void> {
    this.controller?.abort();
    this.controller = undefined;
    // Wait a tick for the loop to unwind so tests/shutdown are deterministic.
    while (this.looping) await new Promise((r) => setTimeout(r, 5));
  }

  private setState(next: DictationState): void {
    const prev = this.current;
    this.current = next;
    const changed =
      prev.online !== next.online ||
      prev.dictating !== next.dictating ||
      prev.busy !== next.busy ||
      prev.status !== next.status;
    if (!changed) return;
    for (const fn of this.listeners) {
      try {
        fn(next);
      } catch {
        // A misbehaving listener must not break the engine.
      }
    }
  }

  private async loop(signal: AbortSignal): Promise<void> {
    this.looping = true;
    let backoff = this.minBackoffMs;
    try {
      while (!signal.aborted) {
        const disc = await readDiscovery(this.controlPath);
        if (!disc.ok) {
          this.setState(offlineState(disc.reason));
          await this.delay(backoff, signal);
          backoff = Math.min(backoff * 2, this.maxBackoffMs);
          continue;
        }

        try {
          await streamEvents(disc.doc, {
            idleTimeoutMs: this.idleTimeoutMs,
            signal,
            onOpen: () => {
              backoff = this.minBackoffMs; // healthy connection resets backoff
              this.log(`connected: ${disc.doc.baseUrl}`);
            },
            // Level-triggered: adopt the latest snapshot from every state/edge event.
            onEvent: (_event, snap) => this.setState(onlineState(snap)),
          });
          // Clean EOF: Scribe closed the stream (e.g. graceful exit).
          if (!signal.aborted) this.setState(offlineState("stream-closed"));
        } catch (err) {
          if (signal.aborted) break;
          const reason: OfflineReason = reasonFromError(err);
          this.setState(offlineState(reason));
          this.log(`stream error: ${reason}`);
        }

        if (signal.aborted) break;
        await this.delay(backoff, signal);
        backoff = Math.min(backoff * 2, this.maxBackoffMs);
      }
    } finally {
      this.looping = false;
    }
  }

  private delay(ms: number, signal: AbortSignal): Promise<void> {
    return new Promise((resolve) => {
      if (signal.aborted) return resolve();
      const onAbort = () => {
        clearTimeout(timer);
        resolve();
      };
      const timer = setTimeout(() => {
        signal.removeEventListener("abort", onAbort);
        resolve();
      }, ms);
      signal.addEventListener("abort", onAbort, { once: true });
    });
  }
}
