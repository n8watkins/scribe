/**
 * Discovery - reading and validating `~/.scribe/control.json` (contract section 5).
 *
 * Resolves how to reach the running Scribe server (baseUrl + readToken +
 * endpoints), and applies the pid liveness kill-switch. A missing, unparseable,
 * too-new, or pid-dead file always resolves to an offline reason, never to a
 * usable target - the first half of the "never stuck dictating" guarantee.
 */

import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import type { OfflineReason } from "./types.js";

/** The major discovery-file `schemaVersion` this consumer was written for. */
export const DISCOVERY_SCHEMA_MAJOR = 1;

/** Parsed `control.json` - only the fields this consumer relies on. */
export interface DiscoveryDoc {
  schemaVersion: number;
  app: string;
  appVersion?: string;
  pid?: number;
  transport?: string;
  baseUrl: string;
  endpoints: Record<string, string>;
  readToken: string;
  updatedAt?: string;
}

export type DiscoveryResult =
  | { ok: true; doc: DiscoveryDoc; path: string }
  | { ok: false; reason: OfflineReason; path: string };

export interface ControlPathOptions {
  /** Read the Dev flavor's `control.dev.json` instead of `control.json`. */
  dev?: boolean;
  /** An explicit override path (wins over `dev`/`home`). */
  path?: string;
  /** Home directory override, mainly for tests. */
  home?: string;
}

/** Resolve the canonical control-file path for the given flavor. */
export function controlFilePath(opts: ControlPathOptions = {}): string {
  if (opts.path) return opts.path;
  const home = opts.home ?? homedir();
  const name = opts.dev ? "control.dev.json" : "control.json";
  return join(home, ".scribe", name);
}

/** `process.kill(pid, 0)` liveness probe. EPERM means alive-but-not-ours. */
export function isProcessAlive(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === "EPERM";
  }
}

function parseDiscovery(raw: unknown): DiscoveryDoc | null {
  if (typeof raw !== "object" || raw === null) return null;
  const o = raw as Record<string, unknown>;
  if (typeof o.schemaVersion !== "number") return null;
  if (typeof o.baseUrl !== "string" || o.baseUrl.length === 0) return null;
  if (typeof o.readToken !== "string" || o.readToken.length === 0) return null;
  const endpoints: Record<string, string> = {};
  if (typeof o.endpoints === "object" && o.endpoints !== null) {
    for (const [k, v] of Object.entries(o.endpoints as Record<string, unknown>)) {
      if (typeof v === "string") endpoints[k] = v;
    }
  }
  return {
    schemaVersion: o.schemaVersion,
    app: typeof o.app === "string" ? o.app : "scribe",
    appVersion: typeof o.appVersion === "string" ? o.appVersion : undefined,
    pid: typeof o.pid === "number" ? o.pid : undefined,
    transport: typeof o.transport === "string" ? o.transport : undefined,
    baseUrl: o.baseUrl,
    endpoints,
    readToken: o.readToken,
    updatedAt: typeof o.updatedAt === "string" ? o.updatedAt : undefined,
  };
}

/**
 * Read and validate the control file. Applies, in order: readability, JSON
 * validity, required fields, schema-major ceiling, then the pid kill-switch.
 * Any failure returns a specific `OfflineReason`.
 */
export async function readDiscovery(path: string): Promise<DiscoveryResult> {
  let text: string;
  try {
    text = await readFile(path, "utf8");
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code;
    return {
      ok: false,
      reason: code === "ENOENT" ? "control-file-missing" : "control-file-unreadable",
      path,
    };
  }

  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch {
    return { ok: false, reason: "control-file-invalid", path };
  }

  const doc = parseDiscovery(json);
  if (!doc) return { ok: false, reason: "control-file-invalid", path };
  if (doc.schemaVersion > DISCOVERY_SCHEMA_MAJOR) {
    return { ok: false, reason: "schema-too-new", path };
  }
  // pid-check discovery itself: a crashed Scribe may leave a stale file behind.
  if (typeof doc.pid === "number" && !isProcessAlive(doc.pid)) {
    return { ok: false, reason: "pid-dead", path };
  }
  return { ok: true, doc, path };
}

/** Build a full request URL for a logical endpoint, honoring the file's map. */
export function endpointUrl(doc: DiscoveryDoc, name: "status" | "events"): string {
  const base = doc.baseUrl.replace(/\/+$/, "");
  const fallback = name === "status" ? "/v1/status" : "/v1/events";
  const path = doc.endpoints[name] ?? fallback;
  return base + (path.startsWith("/") ? path : `/${path}`);
}
