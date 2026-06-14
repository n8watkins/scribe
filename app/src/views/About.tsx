import { useEffect, useState, type ReactNode } from "react";
import { getVersion as getAppVersion } from "@tauri-apps/api/app";
import { check as checkUpdaterPackage } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  ArrowUpRight,
  Check,
  Clock,
  Download,
  ExternalLink,
  FolderOpen,
  History,
  Info,
  Keyboard,
  ListChecks,
  Mic,
  Replace,
  StickyNote,
} from "lucide-react";
import {
  commandErrorMessage,
  getDataDir,
  openDataFolder,
  openReleasePage,
  type UpdateCheckResult,
} from "../backend";
import { detectUpdate } from "../lib/updates";
import type { ViewName } from "../types";
import { Toggle } from "../components/primitives";
import "./about.css";

/** The GitHub releases list — every release with its notes, i.e. the changelog
 * the user can always reference. */
const RELEASES_URL = "https://github.com/n8watkins/scribe/releases";

const FEATURES = [
  {
    Icon: Keyboard,
    title: "Dictate anywhere",
    text: "Hold-to-talk or toggle with a global hotkey — your voice lands as text in any app, no window switching.",
  },
  {
    Icon: Mic,
    title: "Private, on-device Whisper",
    text: "Speech is transcribed locally with Whisper. Your audio never leaves the machine and dictation keeps working offline.",
  },
  {
    Icon: Clock,
    title: "Last Transcript Buffer",
    text: "Your latest dictation stays parked and ready to paste on demand — without ever clobbering your clipboard.",
  },
  {
    Icon: StickyNote,
    title: "Quick notes, smart recap",
    text: "Capture a spoken note with ~ + Q, then let a local model summarise and pull out the action items.",
  },
  {
    Icon: History,
    title: "Searchable history",
    text: "Every transcript is kept in a fast, local, searchable log — with retention limits you control.",
  },
  {
    Icon: Replace,
    title: "Text replacements",
    text: "Names, jargon, and shorthand get fixed automatically with your own find-and-replace rules.",
  },
];

const SETUP_STEPS: {
  title: string;
  text: string;
  action: string;
  target: ViewName;
}[] = [
  {
    title: "Download a model",
    text: "Scribe transcribes with a local Whisper model. Open Models and download one — a smaller model is faster, a larger one is more accurate.",
    action: "Download a model",
    target: "Models",
  },
  {
    title: "Choose a microphone",
    text: "Pick the input device Scribe records from under Audio, then run the level test so you know your voice is coming through.",
    action: "Choose a microphone",
    target: "Audio",
  },
  {
    title: "Set your hotkeys",
    text: "Confirm your hold-to-talk, toggle, and paste-last shortcuts in Hotkeys. Then press your hold-to-talk key anywhere and start dictating.",
    action: "Set your hotkeys",
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

  // Prefer a manual check result if the user just ran one, otherwise fall back
  // to the app's background-poll result so an available update always shows here
  // (even after dismissing the topbar chip).
  const effectiveUpdate = updateResult ?? autoUpdateInfo ?? null;

  const updateStatus =
    updateError ??
    installStatus ??
    (effectiveUpdate
      ? effectiveUpdate.updateAvailable
        ? `You're on an old version — v${effectiveUpdate.latestVersion} is available.`
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
            Scribe is a Windows tray utility for private speech-to-text.
          </p>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Version</strong>
              <span className="about-version">v{version}</span>
            </div>
            <small>
              The packaged application version running on this device.
            </small>
          </div>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Updates</strong>
            </div>
            <small>{updateStatus}</small>
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
              Downloads and installs new versions on launch, with a Scribe screen
              — no Windows installer popups.
            </small>
          </div>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Privacy</strong>
              <span className="pill preserve">Local-first</span>
            </div>
            <small>
              Audio is recorded and transcribed entirely on your device. Once a
              model is downloaded, dictation works offline and nothing is sent
              to any server.
            </small>
          </div>

          <div className="about-block">
            <div className="about-block-head">
              <strong>Local data path</strong>
            </div>
            <small>
              Where Scribe keeps your database, audio clips, and models.
            </small>
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
          </div>
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
            Fast, private voice-to-text that lives in your tray and works in
            every app — here's what you get.
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
            Three steps get you dictating. Each one jumps to the page where you
            set it up.
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
