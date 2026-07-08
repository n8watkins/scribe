/** Shared test helpers: temp control files and small waiters. */

import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

export interface ControlDoc {
  schemaVersion?: number;
  app?: string;
  appVersion?: string;
  pid?: number;
  transport?: string;
  baseUrl?: string;
  endpoints?: Record<string, string>;
  readToken?: string;
  updatedAt?: string;
}

/** Write a control.json into a fresh temp dir; returns the file path + cleanup. */
export async function writeControlFile(
  doc: ControlDoc,
): Promise<{ path: string; cleanup: () => Promise<void> }> {
  const dir = await mkdtemp(join(tmpdir(), "scribe-mcp-test-"));
  const path = join(dir, "control.json");
  await writeFile(path, JSON.stringify(doc), "utf8");
  return { path, cleanup: () => rm(dir, { recursive: true, force: true }) };
}

/** Build a valid control doc pointing at a mock's base URL. */
export function controlFor(baseUrl: string, token: string, extra: ControlDoc = {}): ControlDoc {
  return {
    schemaVersion: 1,
    app: "scribe",
    appVersion: "0.7.0",
    pid: process.pid,
    transport: "http-sse",
    baseUrl,
    endpoints: { status: "/v1/status", events: "/v1/events" },
    readToken: token,
    updatedAt: "2026-07-08T12:00:00.000Z",
    ...extra,
  };
}

/** Poll `predicate` until it is true or `timeoutMs` elapses. */
export async function waitFor(
  predicate: () => boolean,
  { timeoutMs = 2000, intervalMs = 10 }: { timeoutMs?: number; intervalMs?: number } = {},
): Promise<void> {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeoutMs) throw new Error("waitFor: timed out");
    await new Promise((r) => setTimeout(r, intervalMs));
  }
}

export function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
