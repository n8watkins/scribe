import { useEffect, useState } from "react";
import { getVersion as getAppVersion } from "@tauri-apps/api/app";
import { check as checkUpdaterPackage } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  checkForUpdate,
  commandErrorMessage,
  openReleasePage,
  type UpdateCheckResult,
} from "../backend";
import { SectionPanel, SettingRow } from "../components/layout";

export function AboutView() {
  const [version, setVersion] = useState("...");
  const [checking, setChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(
    null,
  );
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installStatus, setInstallStatus] = useState<string | null>(null);

  useEffect(() => {
    void getAppVersion().then(setVersion).catch(() => setVersion("unknown"));
  }, []);

  const handleCheckForUpdate = async () => {
    if (checking) {
      return;
    }
    setChecking(true);
    setUpdateError(null);
    setUpdateResult(null);
    try {
      setUpdateResult(await checkForUpdate());
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
          "The latest release has no signed update package — use View release to download the installer.",
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

  return (
    <section className="view-grid">
      <article className="buffer-card span-2">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Scribe</p>
            <h2>Dictate locally without consuming your clipboard</h2>
          </div>
          <span className="pill preserve">Local-first</span>
        </div>
        <p className="transcript-text">
          Scribe is a Windows tray utility for private speech-to-text. It
          records when you press a global hotkey, transcribes locally with
          Whisper, stores the result in a Last Transcript Buffer, and lets you
          insert it later without permanently overwriting the system clipboard.
        </p>
      </article>

      <SectionPanel title="App details">
        <SettingRow
          description="Current packaged application version."
          label="Version"
        >
          <strong>v{version}</strong>
        </SettingRow>
        <SettingRow
          description={
            updateError ??
            installStatus ??
            (updateResult
              ? updateResult.updateAvailable
                ? `Version ${updateResult.latestVersion} is available (you have ${updateResult.currentVersion}).`
                : `You are on the latest version (${updateResult.currentVersion}).`
              : "Compare this install against the latest GitHub release.")
          }
          label="Updates"
        >
          {updateResult?.updateAvailable ? (
            <>
              <button
                className="primary-button"
                disabled={installing}
                onClick={() => void handleInstallUpdate()}
                type="button"
              >
                {installing ? "Installing..." : "Install update"}
              </button>
              <button
                className="ghost-button"
                disabled={installing}
                onClick={() => void openReleasePage(updateResult.releaseUrl)}
                type="button"
              >
                View release
              </button>
            </>
          ) : (
            <button
              className="ghost-button"
              disabled={checking}
              onClick={() => void handleCheckForUpdate()}
              type="button"
            >
              {checking ? "Checking..." : "Check for updates"}
            </button>
          )}
        </SettingRow>
        <SettingRow
          description="Transcription runs locally after model download."
          label="Privacy"
        >
          <span className="pill preserve">Local-first</span>
        </SettingRow>
        <SettingRow
          description="Default location for app data and models."
          label="Local data path"
        >
          <code>%APPDATA%/com.natkins.scribe/</code>
        </SettingRow>
      </SectionPanel>
    </section>
  );
}
