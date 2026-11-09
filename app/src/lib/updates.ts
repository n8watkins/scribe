import { check } from "@tauri-apps/plugin-updater";
import { getVersion } from "@tauri-apps/api/app";
import { checkForUpdate, type UpdateCheckResult } from "../backend";
import { currentPlatform } from "./platform";

const RELEASES_URL = "https://github.com/n8watkins/scribe/releases";

/**
 * Detect whether an update is available via the updater's `latest.json` endpoint
 * (a GitHub *release asset*, served from the CDN) instead of the GitHub REST API.
 *
 * Why: the REST API (`api.github.com/.../releases/latest`, used by the old
 * `checkForUpdate` command) is capped at ~60 requests/hour for unauthenticated
 * callers, so frequent polling trips a 403 rate limit and silently kills update
 * detection. `latest.json` is not subject to that limit, and it's the exact same
 * source the actual install uses — so detection and install can never disagree.
 *
 * Shaped like `UpdateCheckResult` so existing callers (the topbar chip, About)
 * are unchanged. Network/parse failures propagate to the caller (the poll
 * swallows them; About surfaces them).
 */
export async function detectUpdate(): Promise<UpdateCheckResult> {
  if (!updatesInstallInApp()) {
    // latest.json carries a signed package for Windows only, so the updater
    // plugin can't resolve one here; the backend reads the same manifest's
    // version and the UI links to the release for a manual install.
    return checkForUpdate();
  }
  const currentVersion = await getVersion();
  const update = await check();
  return update
    ? {
        updateAvailable: true,
        currentVersion,
        latestVersion: update.version,
        releaseUrl: RELEASES_URL,
        releaseNotes: update.body?.trim() || null,
      }
    : {
        updateAvailable: false,
        currentVersion,
        latestVersion: currentVersion,
        releaseUrl: RELEASES_URL,
      };
}

/**
 * Whether new versions download and install from inside the app. Only the
 * Windows build publishes a signed updater package; the macOS and Linux betas
 * update by installing the new release by hand.
 */
export function updatesInstallInApp(): boolean {
  return currentPlatform() === "windows";
}
