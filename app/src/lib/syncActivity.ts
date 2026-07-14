import type { AppSettings, GithubSyncActivity } from "../backend";

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

export function formatActivity(
  activity: GithubSyncActivity | null,
  repo: string,
): string {
  if (activity == null || activity.repo !== repo) {
    return "Not run for this repository";
  }

  const source = activity.source === "automatic" ? "Automatic" : "Manual";
  const when = new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(activity.completedAt));
  if (activity.outcome === "error") {
    return `${source} backup failed ${when}`;
  }
  return `${source}: ${formatCount(activity.syncedItems, "item")}, ${formatCount(activity.filesWritten, "file")} on ${when}`;
}

export function syncActivityError(
  activity: GithubSyncActivity | null,
  repo: string,
): string | null {
  if (
    activity == null ||
    activity.repo !== repo ||
    activity.outcome !== "error"
  ) {
    return null;
  }
  return activity.errorMessage ?? "The last GitHub backup did not complete.";
}

export function formatCount(count: number, singular: string): string {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
}
