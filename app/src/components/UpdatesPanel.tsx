import { useState } from "react";
import { check as checkUpdaterPackage } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { Download, ExternalLink, RefreshCw } from "lucide-react";
import {
  commandErrorMessage,
  openReleasePage,
  type UpdateCheckResult,
} from "../backend";
import { forPlatform } from "../lib/platform";
import { detectUpdate, updatesInstallInApp } from "../lib/updates";
import { SectionPanel } from "./layout";
import { Toggle } from "./primitives";
import { ReleaseNotesBody } from "./ReleaseNotes";
import "../views/about.css";

const RELEASES_URL = "https://github.com/n8watkins/scribe/releases";

/** General > Updates: whether Scribe is current, the install or download
 * button when it isn't, and the automatic check and install switches. */
export function UpdatesPanel({
  lastUpdateCheck,
  autoUpdateInfo,
  autoCheckEnabled,
  onToggleAutoCheck,
  autoInstallEnabled,
  onToggleAutoInstall,
}: {
  /** Timestamp (ms) of the last successful background update poll, so the
   * polling is observable here. */
  lastUpdateCheck?: number | null;
  /** Latest auto-detected update from the app's background poll, so an
   * available update always shows here without a manual check. */
  autoUpdateInfo?: UpdateCheckResult | null;
  autoCheckEnabled?: boolean;
  onToggleAutoCheck?: (enabled: boolean) => void;
  autoInstallEnabled?: boolean;
  onToggleAutoInstall?: (enabled: boolean) => void;
}) {
  const [checking, setChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(
    null,
  );
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installStatus, setInstallStatus] = useState<string | null>(null);

  const handleCheckForUpdate = async () => {
    if (checking) {
      return;
    }
    setChecking(true);
    setUpdateError(null);
    setUpdateResult(null);
    try {
      setUpdateResult(await detectUpdate());
    } catch (cause) {
      setUpdateError(commandErrorMessage(cause));
    } finally {
      setChecking(false);
    }
  };

  const handleInstallUpdate = async () => {
    if (installing) {
      return;
    }
    setInstalling(true);
    setUpdateError(null);
    setInstallStatus("Contacting the release server...");
    try {
      const update = await checkUpdaterPackage();
      if (!update) {
        setInstallStatus(null);
        setUpdateError(
          "The latest release has no signed update package. Use View release to download the installer.",
        );
        return;
      }
      let downloaded = 0;
      let total: number | null = null;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? null;
            setInstallStatus("Downloading update...");
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            setInstallStatus(
              total
                ? `Downloading update... ${Math.round((downloaded / total) * 100)}%`
                : "Downloading update...",
            );
            break;
          case "Finished":
            setInstallStatus("Installing... the app will restart.");
            break;
        }
      });
      // On Windows the installer closes and restarts the app itself, so
      // execution rarely gets past downloadAndInstall; relaunch covers the
      // platforms/paths where it does.
      await relaunch();
    } catch (cause) {
      setInstallStatus(null);
      setUpdateError(commandErrorMessage(cause));
    } finally {
      setInstalling(false);
    }
  };

  // Prefer a manual check result if the user just ran one, otherwise fall back
  // to the app's background-poll result so an available update always shows here
  // (even after dismissing the topbar chip).
  const effectiveUpdate = updateResult ?? autoUpdateInfo ?? null;
  const inAppInstall = updatesInstallInApp();

  const updateStatus =
    updateError ??
    installStatus ??
    (effectiveUpdate
      ? effectiveUpdate.updateAvailable
        ? `You're on an old version. v${effectiveUpdate.latestVersion} is available.`
        : "You're on the latest version."
      : "Scribe checks GitHub for new versions.");

  return (
    <SectionPanel
      icon={<RefreshCw aria-hidden="true" size={16} />}
      title="Updates"
    >
          <div className="about-block updates-block">
            <strong className="updates-status">{updateStatus}</strong>
            {effectiveUpdate?.updateAvailable && effectiveUpdate.releaseNotes ? (
              <ReleaseNotesBody
                className="update-notes about-update-notes"
                markdown={effectiveUpdate.releaseNotes}
              />
            ) : null}
            <small className="about-update-poll">
              {autoCheckEnabled === false
                ? "Automatic checks are off"
                : "Checks automatically"}
              {lastUpdateCheck
                ? ` · last checked ${new Date(lastUpdateCheck).toLocaleTimeString()}`
                : ""}
            </small>
            <div className="about-block-actions">
              {effectiveUpdate?.updateAvailable ? (
                inAppInstall ? (
                  <button
                    className="primary-button about-update-button"
                    disabled={installing}
                    onClick={() => void handleInstallUpdate()}
                    type="button"
                  >
                    <Download aria-hidden="true" size={15} />
                    {installing
                      ? "Installing..."
                      : `Install v${effectiveUpdate.latestVersion}`}
                  </button>
                ) : (
                  <button
                    className="primary-button about-update-button"
                    onClick={() => void openReleasePage(effectiveUpdate.releaseUrl)}
                    type="button"
                  >
                    <Download aria-hidden="true" size={15} />
                    {`Download v${effectiveUpdate.latestVersion}`}
                  </button>
                )
              ) : null}
              <button
                className="secondary-button about-update-button"
                disabled={checking}
                onClick={() => void handleCheckForUpdate()}
                type="button"
              >
                {checking ? "Checking..." : "Check for updates"}
              </button>
              <button
                className="ghost-button about-update-button"
                onClick={() => void openReleasePage(RELEASES_URL)}
                type="button"
              >
                <ExternalLink aria-hidden="true" size={15} />
                View releases
              </button>
            </div>
            <div className="about-autocheck">
              <span>Automatically check for updates</span>
              <Toggle
                checked={autoCheckEnabled ?? true}
                label="Automatically check for updates"
                onChange={(enabled) => onToggleAutoCheck?.(enabled)}
              />
            </div>
            {inAppInstall ? (
              <>
                <div className="about-autocheck about-autoinstall">
                  <span>Install updates automatically</span>
                  <Toggle
                    checked={autoInstallEnabled ?? true}
                    disabled={autoCheckEnabled === false}
                    label="Install updates automatically"
                    onChange={(enabled) => onToggleAutoInstall?.(enabled)}
                  />
                </div>
                <small className="about-autoinstall-hint">
                  Downloads and installs new versions on launch, with a Scribe screen and no Windows installer popups.
                </small>
              </>
            ) : (
              <small className="about-autoinstall-hint">
                {forPlatform({
                  windows: "",
                  macos:
                    "The macOS beta doesn't install updates itself yet. Download the new version and drag it into Applications to replace this one.",
                  linux:
                    "The Linux beta doesn't install updates itself yet. Download the new .deb or AppImage and install it over this one.",
                })}
              </small>
            )}
          </div>

    </SectionPanel>
  );
}
