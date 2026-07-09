/**
 * Wire types and view types for the Scribe dictation-state contract (v1).
 *
 * `ScribeSnapshot` is the section-1 payload exactly as received from Scribe.
 * `DictationState` is the normalized view this MCP server exposes to clients:
 * it ALWAYS resolves to not-dictating whenever Scribe is unreachable, stale, or
 * dead, so a crashed producer can never wedge a consumer into a stuck
 * "dictating" belief (contract sections 5 + 7).
 */

/** The major snapshot `schemaVersion` this consumer was written for. */
export const SNAPSHOT_SCHEMA_MAJOR = 1;

/** The canonical Scribe state snapshot - contract section 1. */
export interface ScribeSnapshot {
  schemaVersion: number;
  app: string;
  appVersion?: string;
  status: string;
  dictating: boolean;
  busy: boolean;
  since?: string;
  updatedAt?: string;
  pid?: number;
}

/**
 * Why the MCP server currently considers Scribe not-dictating. Present only on
 * offline states. Every value here maps to "not dictating" per the contract's
 * hard liveness requirement (section 7).
 */
export type OfflineReason =
  | "not-connected-yet"
  | "control-file-missing"
  | "control-file-unreadable"
  | "control-file-invalid"
  | "schema-too-new"
  | "pid-dead"
  | "connection-refused"
  | "http-error"
  | "stream-closed"
  | "stream-timeout";

/**
 * The normalized dictation state this server hands to MCP clients. `dictating`
 * and `busy` are authoritative: they are false whenever `online` is false.
 */
export interface DictationState {
  /** Could we reach a live Scribe and read a fresh, in-contract snapshot? */
  online: boolean;
  /** Narrow flag: mic actively capturing. Always false when offline. */
  dictating: boolean;
  /** Broad flag: inside a dictation cycle. Always false when offline. */
  busy: boolean;
  /** Scribe's raw PascalCase status, or "Offline" when unreachable. */
  status: string;
  /** When the current status was entered, or null when offline/unknown. */
  since: string | null;
  /** Scribe's snapshot `updatedAt`, or the observation time when offline. */
  updatedAt: string;
  /** Scribe's pid, or null when offline. */
  pid: number | null;
  /** Constant "scribe" when online; null when offline. */
  app: string | null;
  /** Scribe semver when online; null when offline. */
  appVersion: string | null;
  /** Snapshot schema version when online; null when offline. */
  schemaVersion: number | null;
  /** Present only when offline: why we report not-dictating. */
  reason?: OfflineReason;
  /** When THIS server produced this view. Always fresh. */
  observedAt: string;
}

/** Validate and normalize an untrusted value into a `ScribeSnapshot`.
 *
 * Rejects (returns null) anything missing the required typed fields, or whose
 * major schemaVersion is newer than we understand. Unknown additive fields are
 * ignored, never a failure (contract section 0).
 */
export function parseSnapshot(raw: unknown): ScribeSnapshot | null {
  if (typeof raw !== "object" || raw === null) return null;
  const o = raw as Record<string, unknown>;
  if (typeof o.schemaVersion !== "number") return null;
  if (o.schemaVersion > SNAPSHOT_SCHEMA_MAJOR) return null;
  if (typeof o.status !== "string") return null;
  if (typeof o.dictating !== "boolean") return null;
  if (typeof o.busy !== "boolean") return null;
  return {
    schemaVersion: o.schemaVersion,
    app: typeof o.app === "string" ? o.app : "scribe",
    appVersion: typeof o.appVersion === "string" ? o.appVersion : undefined,
    status: o.status,
    dictating: o.dictating,
    busy: o.busy,
    since: typeof o.since === "string" ? o.since : undefined,
    updatedAt: typeof o.updatedAt === "string" ? o.updatedAt : undefined,
    pid: typeof o.pid === "number" ? o.pid : undefined,
  };
}

/** Build an online `DictationState` from a validated snapshot. */
export function onlineState(snap: ScribeSnapshot): DictationState {
  const now = new Date().toISOString();
  const dictating = snap.dictating === true;
  // Enforce the hard invariant defensively: dictating implies busy.
  const busy = snap.busy === true || dictating;
  return {
    online: true,
    dictating,
    busy,
    status: snap.status,
    since: snap.since ?? null,
    updatedAt: snap.updatedAt ?? now,
    pid: snap.pid ?? null,
    app: snap.app ?? null,
    appVersion: snap.appVersion ?? null,
    schemaVersion: snap.schemaVersion ?? null,
    observedAt: now,
  };
}

/** Build an offline `DictationState`. Always not-dictating, not-busy. */
export function offlineState(reason: OfflineReason): DictationState {
  const now = new Date().toISOString();
  return {
    online: false,
    dictating: false,
    busy: false,
    status: "Offline",
    since: null,
    updatedAt: now,
    pid: null,
    app: null,
    appVersion: null,
    schemaVersion: null,
    reason,
    observedAt: now,
  };
}
