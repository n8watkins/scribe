/**
 * A tiny mock of Scribe's HTTP+SSE interface for tests (contract sections 3+4).
 * Serves `GET /v1/status` and `GET /v1/events`, enforces the bearer/query token,
 * replays the current snapshot on SSE connect, and lets tests push transitions.
 */

import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { AddressInfo } from "node:net";

export interface Snapshot {
  schemaVersion: number;
  app: string;
  appVersion: string;
  status: string;
  dictating: boolean;
  busy: boolean;
  since: string;
  updatedAt: string;
  pid: number;
}

export function snapshot(partial: Partial<Snapshot> = {}): Snapshot {
  return {
    schemaVersion: 1,
    app: "scribe",
    appVersion: "0.7.0",
    status: "Idle",
    dictating: false,
    busy: false,
    since: "2026-07-08T12:00:00.000Z",
    updatedAt: "2026-07-08T12:00:00.000Z",
    pid: process.pid,
    ...partial,
  };
}

export class MockScribe {
  private server: Server;
  private current: Snapshot;
  private clients = new Set<ServerResponse>();
  readonly token: string;
  private port = 0;

  constructor(opts: { token?: string; initial?: Partial<Snapshot> } = {}) {
    this.token = opts.token ?? "test-token-abc";
    this.current = snapshot(opts.initial);
    this.server = createServer((req, res) => this.handle(req, res));
  }

  async listen(): Promise<void> {
    await new Promise<void>((resolve) => this.server.listen(0, "127.0.0.1", resolve));
    this.port = (this.server.address() as AddressInfo).port;
  }

  get baseUrl(): string {
    return `http://127.0.0.1:${this.port}`;
  }

  /** Number of currently attached SSE clients. */
  get streamCount(): number {
    return this.clients.size;
  }

  private authOk(req: IncomingMessage): boolean {
    const header = req.headers["authorization"];
    if (header === `Bearer ${this.token}`) return true;
    const url = new URL(req.url ?? "/", this.baseUrl);
    return url.searchParams.get("token") === this.token;
  }

  private handle(req: IncomingMessage, res: ServerResponse): void {
    const url = new URL(req.url ?? "/", this.baseUrl);
    if (!this.authOk(req)) {
      res.writeHead(401, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "unauthorized" }));
      return;
    }
    if (url.pathname === "/v1/status") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(this.current));
      return;
    }
    if (url.pathname === "/v1/events") {
      res.writeHead(200, {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        connection: "keep-alive",
      });
      this.clients.add(res);
      res.on("close", () => this.clients.delete(res));
      // Immediate replay of the current snapshot as a `state` event.
      this.writeEvent(res, "state", this.current);
      return;
    }
    res.writeHead(404).end();
  }

  private writeEvent(res: ServerResponse, event: string, data: Snapshot): void {
    res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
  }

  /** Push a raw named event carrying the given snapshot to all SSE clients. */
  emit(event: string, snap: Snapshot): void {
    this.current = snap;
    for (const res of this.clients) this.writeEvent(res, event, snap);
  }

  /** Convenience: transition to Recording and fire `state` + `dictation.started`. */
  startDictation(overrides: Partial<Snapshot> = {}): void {
    const snap = snapshot({ status: "Recording", dictating: true, busy: true, ...overrides });
    this.current = snap;
    for (const res of this.clients) {
      this.writeEvent(res, "state", snap);
      this.writeEvent(res, "dictation.started", snap);
    }
  }

  /** Convenience: transition to Idle and fire `state` + `dictation.stopped`. */
  stopDictation(overrides: Partial<Snapshot> = {}): void {
    const snap = snapshot({ status: "Idle", dictating: false, busy: false, ...overrides });
    this.current = snap;
    for (const res of this.clients) {
      this.writeEvent(res, "state", snap);
      this.writeEvent(res, "dictation.stopped", snap);
    }
  }

  /** Emit a heartbeat comment line, like Scribe's `: ping`. */
  ping(): void {
    for (const res of this.clients) res.write(`: ping\n\n`);
  }

  setSnapshot(snap: Snapshot): void {
    this.current = snap;
  }

  /** Forcibly drop all open SSE streams (simulates a mid-stream crash). */
  dropStreams(): void {
    for (const res of this.clients) res.destroy();
    this.clients.clear();
  }

  async close(): Promise<void> {
    this.dropStreams();
    await new Promise<void>((resolve, reject) =>
      this.server.close((err) => (err ? reject(err) : resolve())),
    );
  }
}
