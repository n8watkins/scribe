import { useEffect, useState } from "react";
import { getVersion as getAppVersion } from "@tauri-apps/api/app";
import { check as checkUpdaterPackage } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  Check,
  Clock,
  Download,
  ExternalLink,
  FolderOpen,
  History,
  Keyboard,
  Mic,
  Replace,
  StickyNote,
} from "lucide-react";
import {
  checkForUpdate,
  commandErrorMessage,
  getDataDir,
  openDataFolder,
  openReleasePage,
  type UpdateCheckResult,
} from "../backend";
import { SectionPanel } from "../components/layout";
import "./about.css";

const FEATURES = [
  {
    Icon: Keyboard,
    title: "Global hotkeys",
    text: "Hold-to-talk or toggle dictation from anywhere with system-wide shortcuts.",
  },
  {
    Icon: Mic,
    title: "Local Whisper transcription",
    text: "Speech is transcribed on-device with Whisper — no audio leaves your machine.",
  },
  {
    Icon: Clock,
    title: "Last Transcript Buffer",
    text: "Your most recent dictation is held ready to paste, without overwriting the clipboard.",
  },
  {
    Icon: StickyNote,
    title: "Quick notes",
    text: "Capture a dictated note with ~ + Q and have it analysed by a local model.",
  },
  {
    Icon: History,
    title: "Searchable history",
    text: "Keep a local, searchable record of past transcripts with optional retention limits.",
  },
  {
    Icon: Replace,
    title: "Text replacements",
    text: "Auto-fix names, jargon, and shorthand with custom find-and-replace rules.",
  },
];

const SETUP_STEPS = [
  {
    title: "Download a model",
    text: "Open Models and download a Whisper model so Scribe can transcribe locally.",
  },
  {
    title: "Pick a microphone",
    text: "Choose your input device under Audio so recordings capture the right source.",
  },
  {
    title: "Set your hotkeys",
    text: "Confirm hold-to-talk, toggle, and paste-last shortcuts in Hotkeys, then start dictating.",
  },
];

export function AboutView() {
  const [version, setVersion] = useState("...");
  const [dataDir, setDataDir] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(
    null,
  );
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installStatus, setInstallStatus] = useState<string | null>(null);

  useEffect(() => {
    void getAppVersion().then(setVersion).catch(() => setVersion("unknown"));
    void getDataDir().then(setDataDir).catch(() => setDataDir(null));
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

  const updateStatus =
    updateError ??
    installStatus ??
    (updateResult
      ? updateResult.updateAvailable
        ? `Version ${updateResult.latestVersion} is available (you have ${updateResult.currentVersion}).`
        : `You are on the latest version (${updateResult.currentVersion}).`
      : "Compare this install against the latest GitHub release.");

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

      <SectionPanel title="What Scribe does">
        <ul className="about-features">
          {FEATURES.map(({ Icon, title, text }) => (
            <li key={title}>
              <Icon aria-hidden="true" size={15} />
              <span>
                <strong>{title}</strong> — {text}
              </span>
            </li>
          ))}
        </ul>
      </SectionPanel>

      <SectionPanel
        icon={<Check aria-hidden="true" size={16} />}
        title="Setup"
      >
        <ol className="about-steps">
          {SETUP_STEPS.map(({ title, text }) => (
            <li key={title}>
              <span className="about-step-body">
                <strong>{title}</strong>
                <small>{text}</small>
              </span>
            </li>
          ))}
        </ol>
      </SectionPanel>

      <SectionPanel title="App details">
        <div className="about-block">
          <div className="about-block-head">
            <strong>Version</strong>
            <span className="about-version">v{version}</span>
          </div>
          <small>The packaged application version running on this device.</small>
        </div>

        <div className="about-block">
          <div className="about-block-head">
            <strong>Updates</strong>
          </div>
          <small>{updateStatus}</small>
          <div className="about-block-actions">
            {updateResult?.updateAvailable ? (
              <>
                <button
                  className="primary-button about-update-button"
                  disabled={installing}
                  onClick={() => void handleInstallUpdate()}
                  type="button"
                >
                  <Download aria-hidden="true" size={15} />
                  {installing ? "Installing..." : "Install update"}
                </button>
                <button
                  className="secondary-button about-update-button"
                  disabled={installing}
                  onClick={() => void openReleasePage(updateResult.releaseUrl)}
                  type="button"
                >
                  <ExternalLink aria-hidden="true" size={15} />
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
          </div>
        </div>

        <div className="about-block">
          <div className="about-block-head">
            <strong>Privacy</strong>
            <span className="pill preserve">Local-first</span>
          </div>
          <small>
            Audio is recorded and transcribed entirely on your device. Once a
            model is downloaded, dictation works offline and nothing is sent to
            any server.
          </small>
        </div>

        <div className="about-block">
          <div className="about-block-head">
            <strong>Local data path</strong>
          </div>
          <small>Where Scribe keeps your database, audio clips, and models.</small>
          <div className="about-path-row">
            <code className="about-path" title={dataDir ?? undefined}>
              {dataDir ?? "Loading..."}
            </code>
            <button
              aria-label="Open data folder"
              className="icon-button"
              onClick={() => void openDataFolder()}
              type="button"
            >
              <FolderOpen aria-hidden="true" size={15} />
            </button>
          </div>
        </div>
      </SectionPanel>
    </section>
  );
}
