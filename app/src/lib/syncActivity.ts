import type { AppSettings, GithubSyncReport } from "../backend";

const STORAGE_KEY = "scribe.github.last-manual-sync";

export type SyncActivity =
  | {
      completedAt: string;
      filesWritten: number;
      itemCount: number;
      outcome: "success";
      version: 1;
    }
  | {
      completedAt: string;
      outcome: "error";
      version: 1;
    };

function isSyncActivity(value: unknown): value is SyncActivity {
  if (typeof value !== "object" || value == null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  if (
    candidate.version !== 1 ||
    (candidate.outcome !== "success" && candidate.outcome !== "error") ||
    typeof candidate.completedAt !== "string" ||
    !Number.isFinite(Date.parse(candidate.completedAt))
  ) {
    return false;
  }

  return (
    candidate.outcome === "error" ||
    (typeof candidate.filesWritten === "number" &&
      Number.isSafeInteger(candidate.filesWritten) &&
      candidate.filesWritten >= 0 &&
      typeof candidate.itemCount === "number" &&
      Number.isSafeInteger(candidate.itemCount) &&
      candidate.itemCount >= 0)
  );
}

export function loadSyncActivity(): SyncActivity | null {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored == null) {
      return null;
    }

    const parsed: unknown = JSON.parse(stored);
    if (isSyncActivity(parsed)) {
      return parsed;
    }

    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // A disabled or full storage area must never break backup controls.
  }
  return null;
}

export function rememberSyncSuccess(report: GithubSyncReport): SyncActivity {
  const activity: SyncActivity = {
    completedAt: new Date().toISOString(),
    filesWritten: report.filesWritten,
    itemCount: report.syncedNotes,
    outcome: "success",
    version: 1,
  };
  storeActivity(activity);
  return activity;
}

export function rememberSyncError(): SyncActivity {
  const activity: SyncActivity = {
    completedAt: new Date().toISOString(),
    outcome: "error",
    version: 1,
  };
  storeActivity(activity);
  return activity;
}

export function clearSyncActivity(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Storage is optional; the in-memory caller state is still cleared.
  }
}

export function backupReadiness(
  settings: Pick<AppSettings, "githubSyncAllTranscripts" | "githubSyncEnabled">,
  repoValid: boolean,
): string {
  if (!repoValid) {
    return "Set a valid repository";
  }
  if (!settings.githubSyncEnabled && !settings.githubSyncAllTranscripts) {
    return "Choose what to back up";
  }
  return "Connect GitHub";
}

export function backupCoverage(
  settings: Pick<AppSettings, "githubSyncAllTranscripts" | "githubSyncEnabled">,
): string {
  if (settings.githubSyncEnabled && settings.githubSyncAllTranscripts) {
    return "Notes and dictations";
  }
  if (settings.githubSyncEnabled) {
    return "Notes only";
  }
  if (settings.githubSyncAllTranscripts) {
    return "Dictations only";
  }
  return "Nothing selected";
}

export function formatActivity(activity: SyncActivity | null): string {
  if (activity == null) {
    return "Not run on this device";
  }

  const when = new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(activity.completedAt));
  if (activity.outcome === "error") {
    return `Failed ${when}`;
  }
  return `${formatCount(activity.itemCount, "item")}, ${formatCount(activity.filesWritten, "file")} on ${when}`;
}

export function formatCount(count: number, singular: string): string {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
}

function storeActivity(activity: SyncActivity): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(activity));
  } catch {
    // Storage is optional; a successful sync must stay successful if it fails.
  }
}
