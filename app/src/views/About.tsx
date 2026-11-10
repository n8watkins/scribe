import { useEffect, useState, type ReactNode } from "react";
import { getVersion as getAppVersion } from "@tauri-apps/api/app";
import { check as checkUpdaterPackage } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { forPlatform } from "../lib/platform";
import {
  ArrowUpRight,
  Check,
  Clock,
  Download,
  ExternalLink,
  History,
  Info,
  Keyboard,
  ListChecks,
  Mic,
  Replace,
  ScrollText,
  StickyNote,
} from "lucide-react";
import {
  commandErrorMessage,
  openReleasePage,
  type UpdateCheckResult,
} from "../backend";
import { detectUpdate, updatesInstallInApp } from "../lib/updates";
import type { ViewName } from "../types";
import { Toggle } from "../components/primitives";
import {
  ReleaseHistory,
  ReleaseNotesBody,
} from "../components/ReleaseNotes";
import { CopyDiagnostics } from "../components/CopyDiagnostics";
import "./about.css";

/** The GitHub releases list — every release with its notes, i.e. the changelog
 * the user can always reference. */
const RELEASES_URL = "https://github.com/n8watkins/scribe/releases";

const FEATURES = [
  {
    Icon: Keyboard,
    title: "Dictate anywhere",
    text: "Hold the hotkey, talk, let go. The text lands at your cursor, in any app.",
  },
  {
    Icon: Mic,
    title: "Runs on your computer",
    text: "Whisper or Parakeet transcribes on this machine. Your audio never leaves it, and dictation works offline.",
  },
  {
    Icon: Clock,
    title: "Paste it again",
    text: "Your last dictation stays one hotkey away. Scribe puts your clipboard back after every paste.",
  },
  {
    Icon: StickyNote,
    title: "Quick notes",
    text: "Hold the toggle key and tap Q to save a spoken note instead of pasting it. Local AI can sum it up and pull out the to-dos.",
  },
  {
    Icon: History,
    title: "Searchable history",
    text: "Every dictation is kept on this device, searchable by text and date, for as long as you choose.",
  },
  {
    Icon: Replace,
    title: "Your words",
    text: "A context hint teaches Scribe your names and jargon; replacements fix the mistakes it keeps making.",
  },
];

const SETUP_STEPS: {
  title: string;
  text: string;
  action: string;
  target: ViewName;
}[] = [
  {
    title: "Download a speech model",
    text: "Pick one on Models and language. Base English is small and quick; Small English and Parakeet v3 get more of your words right.",
    action: "Open Models and language",
    target: "Models & language",
  },
  {
    title: "Pick your microphone",
    text: "Choose it on Microphone and record a short test to hear yourself back.",
    action: "Open Microphone",
    target: "Microphone",
  },
  {
    title: "Try your hotkeys",
    text: "Check hold to talk and paste last on Hotkeys, then click into any text box, hold the key and talk.",
    action: "Open Hotkeys",
    target: "Hotkeys",
  },
];

export function AboutView({
  setActiveView,
  lastUpdateCheck,
  autoUpdateInfo,
  autoCheckEnabled,
  onToggleAutoCheck,
  autoInstallEnabled,
  onToggleAutoInstall,
}: {
  setActiveView?: (view: ViewName) => void;
  /** Timestamp (ms) of the last successful background update poll, so the
   * polling is observable here. */
  lastUpdateCheck?: number | null;
  /** Latest auto-detected update from the app's background poll. Lets About
   * always reflect an available update without the user manually checking. */
  autoUpdateInfo?: UpdateCheckResult | null;
  /** Whether background update checks are on (the toggle state). */
  autoCheckEnabled?: boolean;
  /** Flip the "automatically check for updates" setting. */
  onToggleAutoCheck?: (enabled: boolean) => void;
  /** Whether updates install silently on launch (the toggle state). */
  autoInstallEnabled?: boolean;
  /** Flip the "install updates automatically" setting. */
  onToggleAutoInstall?: (enabled: boolean) => void;
}) {
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
      : "Compare this install against the latest GitHub release.");

  const tabs: {
    id: string;
    title: string;
    icon: ReactNode;
    render: () => ReactNode;
  }[] = [
    {
      id: "app-details",
      title: "App details",
      icon: <Info aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card">
          <div className="settings-list">
          <p className="about-lead">
            {forPlatform({
              windows: "Scribe is a Windows tray utility for private speech-to-text.",
              macos: "Scribe is a macOS menu bar utility for private speech-to-text.",
              linux: "Scribe is a Linux tray utility for private speech-to-text.",
            })}
          </p>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Version</strong>
              <span className="about-version">v{version}</span>
            </div>
          </div>

          <div className="about-block" data-setting="Updates">
            <div className="about-block-head">
              <strong>Updates</strong>
            </div>
            <small>{updateStatus}</small>
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

          <div className="about-block">
            <div className="about-block-head">
              <strong>Privacy</strong>
              <span className="pill preserve">Local-first</span>
            </div>
            <small>
              Your audio is recorded and transcribed on this device and never
              uploaded. Your files are listed under Your data.
            </small>
          </div>

          <CopyDiagnostics />
          </div>
        </article>
      ),
    },
    {
      id: "release-notes",
      title: "Release notes",
      icon: <ScrollText aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card about-releases-card">
          <ReleaseHistory />
        </article>
      ),
    },
    {
      id: "features",
      title: "What Scribe does",
      icon: <ListChecks aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card about-features-card">
          <p className="about-lead">
            Private speech-to-text that works in every app and never sends
            your voice anywhere.
          </p>
          <ul className="feature-grid">
            {FEATURES.map(({ Icon, title, text }) => (
              <li className="feature-card" key={title}>
                <span className="feature-card-icon" aria-hidden="true">
                  <Icon size={18} />
                </span>
                <strong className="feature-card-title">{title}</strong>
                <span className="feature-card-text">{text}</span>
              </li>
            ))}
          </ul>
        </article>
      ),
    },
    {
      id: "setup",
      title: "Setup",
      icon: <Check aria-hidden="true" size={16} />,
      render: () => (
        <article className="panel-card">
          <div className="settings-list">
          <p className="about-lead">
            Three steps and you're dictating.
          </p>
          <ol className="about-steps">
            {SETUP_STEPS.map(({ title, text, action, target }) => (
              <li key={title}>
                <span className="about-step-body">
                  <strong>{title}</strong>
                  <small>{text}</small>
                  <button
                    className="secondary-button about-step-button"
                    disabled={!setActiveView}
                    onClick={() => setActiveView?.(target)}
                    type="button"
                  >
                    {action}
                    <ArrowUpRight aria-hidden="true" size={15} />
                  </button>
                </span>
              </li>
            ))}
          </ol>
          </div>
        </article>
      ),
    },
  ];

  const [activeTab, setActiveTab] = useState(tabs[0].id);
  const active = tabs.find((tab) => tab.id === activeTab) ?? tabs[0];

  return (
    <section className="view-grid">
      <div className="settings-tabs" role="tablist" aria-label="About sections">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            id={`about-tab-${tab.id}`}
            aria-controls={`about-panel-${tab.id}`}
            aria-selected={tab.id === active.id}
            className={`settings-tab${tab.id === active.id ? " is-active" : ""}`}
            data-setting={tab.title}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.icon}
            <span>{tab.title}</span>
          </button>
        ))}
      </div>
      <div
        className="settings-tabpanel"
        role="tabpanel"
        id={`about-panel-${active.id}`}
        aria-labelledby={`about-tab-${active.id}`}
      >
        {active.render()}
      </div>
    </section>
  );
}
