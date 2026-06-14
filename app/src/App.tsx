import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getName as getAppName } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { check as checkUpdaterPackage } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  BarChart3,
  Cloud,
  Database,
  Download,
  Eraser,
  Gauge,
  History as HistoryIcon,
  Info,
  Keyboard,
  Mic,
  Minus,
  MonitorCog,
  NotebookPen,
  Radio,
  Settings as SettingsIcon,
  ShieldCheck,
  Square,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  cancelRecording,
  clearLastTranscript,
  commandErrorMessage,
  copyLastTranscript,
  getDashboardData,
  listMicrophones,
  listModels,
  pasteLastTranscript,
  checkForUpdate,
  startRecording,
  stopRecording,
  transcribeRecording,
  updateSettings,
  type AppStateSnapshot,
  type UpdateCheckResult,
  type DashboardData,
  type DictationResult,
  type HotkeyRegistrationFailedEvent,
  type MicrophoneInfo,
  type ModelDownloadProgress,
  type ModelInfo,
  type OutputResult,
  type PartialTranscriptEvent,
  type RecordingResult,
  type RecordingSessionInfo,
  type RecordingErrorEvent,
} from "./backend";
import { playStartCue, playStopCue } from "./sounds";
import "./App.css";
import type {
  LoadState,
  SettingsPatch,
  ToastNotice,
  ViewActions,
  ViewName,
} from "./types";
import { canStartRecording, formatHotkey, routeToView } from "./lib/format";
import { ErrorPanel, InlineError, LoadingPanel, Toast } from "./components/feedback";
import {
  UpdateOverlay,
  type UpdateOverlayState,
} from "./components/UpdateOverlay";
import { DashboardView } from "./views/Dashboard";
import { TranscribeView } from "./views/Transcribe";
import { HistoryView } from "./views/History";
import { StatsView } from "./views/Stats";
import { SettingsView } from "./views/Settings";
import { DataPrivacyView } from "./views/DataPrivacy";
import { HotkeysView } from "./views/Hotkeys";
import { ModelsView } from "./views/Models";
import { AudioView } from "./views/Audio";
import { AboutView } from "./views/About";
import { DeveloperView } from "./views/Developer";
import { SyncView } from "./views/Sync";
import scribeIcon from "./assets/scribe-icon.png";

const navItems: { label: ViewName; Icon: LucideIcon }[] = [
  { label: "Dashboard", Icon: Gauge },
  { label: "Transcribe", Icon: Mic },
  { label: "History", Icon: HistoryIcon },
  { label: "Notes", Icon: NotebookPen },
  { label: "Sync", Icon: Cloud },
  { label: "Stats", Icon: BarChart3 },
  { label: "Settings", Icon: SettingsIcon },
  { label: "Data & Privacy", Icon: ShieldCheck },
  { label: "Hotkeys", Icon: Keyboard },
  { label: "Models", Icon: Database },
  { label: "Audio", Icon: Radio },
  { label: "About", Icon: Info },
];

/** Fires an OS notification that an update is available, so the user sees it
 * even when Scribe is minimized to the tray (the in-app topbar button can't
 * reach a backgrounded window). Best-effort: requests notification permission
 * once if needed and silently no-ops if denied or unavailable. */
async function notifyUpdateAvailable(version: string) {
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (granted) {
      sendNotification({
        title: "Scribe update available",
        body: `Version ${version} is ready to install. Open Scribe → About to update.`,
      });
    }
  } catch {
    // Notifications can be unavailable (permissions, platform); fail quietly.
  }
}

/** Custom window title bar (the main window runs with `decorations: false`).
 * The bar itself is the OS drag region (`data-tauri-drag-region`); the three
 * controls drive the native window. Replaces the native chrome so there's no
 * second "Scribe" in the top-left. */
function TitleBar({ isDev }: { isDev: boolean }) {
  const win = getCurrentWindow();
  return (
    <div className="titlebar" data-tauri-drag-region>
      <span className="titlebar-brand" data-tauri-drag-region>
        Scribe{isDev ? " Dev" : ""}
      </span>
      <div className="window-controls">
        <button
          aria-label="Minimize"
          className="win-ctl"
          onClick={() => void win.minimize()}
          type="button"
        >
          <Minus aria-hidden="true" size={15} />
        </button>
        <button
          aria-label="Maximize"
          className="win-ctl"
          onClick={() => void win.toggleMaximize()}
          type="button"
        >
          <Square aria-hidden="true" size={12} />
        </button>
        <button
          aria-label="Close"
          className="win-ctl win-close"
          onClick={() => void win.close()}
          type="button"
        >
          <X aria-hidden="true" size={16} />
        </button>
      </div>
    </div>
  );
}

function App() {
  const [activeView, setActiveView] = useState<ViewName>("Dashboard");
  const [settingsTabId, setSettingsTabId] = useState<string | null>(null);
  const [dashboardData, setDashboardData] = useState<DashboardData | null>(
    null,
  );
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [savingSettings, setSavingSettings] = useState(false);
  const [clearingLastTranscript, setClearingLastTranscript] = useState(false);
  const [pastingLastTranscript, setPastingLastTranscript] = useState(false);
  const [copyingLastTranscript, setCopyingLastTranscript] = useState(false);
  const [recordingBusy, setRecordingBusy] = useState(false);
  const [microphones, setMicrophones] = useState<MicrophoneInfo[] | null>(null);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [toast, setToast] = useState<ToastNotice | null>(null);
  const [isDevFlavor, setIsDevFlavor] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResult | null>(null);
  // Drives the branded auto-update screen. Non-null while an on-launch silent
  // install is downloading/installing (or showing its error/Continue state);
  // null hides the overlay and returns to the app.
  const [autoUpdateState, setAutoUpdateState] =
    useState<UpdateOverlayState | null>(null);
  // Timestamp of the last successful auto poll, surfaced in About so the polling
  // is observable (the "last checked" line ticks each cycle).
  const [lastUpdateCheck, setLastUpdateCheck] = useState<number | null>(null);
  // Mirror the auto-update-check setting into a ref so the long-lived poll
  // effect (deps: []) honors the current value without re-subscribing. Defaults
  // to on until settings load.
  const autoUpdateCheckRef = useRef(true);
  autoUpdateCheckRef.current =
    dashboardData?.settings.autoUpdateCheckEnabled ?? true;
  // Mirror the auto-install setting the same way. Defaults to on until settings
  // load; the on-launch flow only proceeds when BOTH this and the check toggle
  // are on (and waits for settings to actually load first).
  const autoInstallUpdatesRef = useRef(true);
  autoInstallUpdatesRef.current =
    dashboardData?.settings.autoInstallUpdates ?? true;
  // One-shot guard: the silent auto-install runs at most once per launch (and
  // never loops, even if the download fails and we fall back to the chip).
  const autoInstallAttemptedRef = useRef(false);
  // Only auto-install once the user's actual settings have loaded — until then
  // the toggle refs hold their (on) defaults, and we don't want to silently
  // install against a saved opt-out that simply hasn't been read yet.
  const settingsLoadedRef = useRef(false);
  if (dashboardData) {
    settingsLoadedRef.current = true;
  }

  // The dev flavor labels itself so two running instances are tellable apart.
  useEffect(() => {
    void getAppName()
      .then((name) => setIsDevFlavor(name.includes("Dev")))
      .catch(() => {});
  }, []);
  const [liveTranscript, setLiveTranscript] =
    useState<PartialTranscriptEvent | null>(null);
  const soundsEnabledRef = useRef(false);
  const notificationsEnabledRef = useRef(false);

  // The Developer panel is opt-in (Settings -> App behavior). Insert it just
  // before About when enabled so the diagnostics sit at the end of the list.
  const developerEnabled =
    dashboardData?.settings.developerSettingsEnabled ?? false;
  const visibleNavItems: { label: ViewName; Icon: LucideIcon }[] =
    developerEnabled
      ? [
          ...navItems.slice(0, navItems.length - 1),
          { label: "Developer", Icon: MonitorCog },
          navItems[navItems.length - 1],
        ]
      : navItems;

  useEffect(() => {
    soundsEnabledRef.current = dashboardData?.settings.soundsEnabled ?? false;
  }, [dashboardData?.settings.soundsEnabled]);

  useEffect(() => {
    notificationsEnabledRef.current =
      dashboardData?.settings.notificationsEnabled ?? false;
  }, [dashboardData?.settings.notificationsEnabled]);

  // If the Developer panel is turned off while it is the active view, fall back
  // to the Dashboard so the user isn't stranded on a now-hidden page.
  useEffect(() => {
    if (activeView === "Developer" && !developerEnabled) {
      setActiveView("Dashboard");
    }
  }, [activeView, developerEnabled]);

  // A deep-link tab (from `openSettings(tab)`) is one-shot: once the user leaves
  // Settings, clear it so re-entering Settings via the sidebar opens the default
  // (last) tab instead of reopening the stale deep-linked one. Guarded on the
  // current value so this can't loop.
  useEffect(() => {
    if (activeView !== "Settings" && settingsTabId !== null) {
      setSettingsTabId(null);
    }
  }, [activeView, settingsTabId]);

  // Read `notificationsEnabled` from a ref (kept in sync above) rather than from
  // settings directly, so toggling notifications doesn't change this callback's
  // identity — otherwise the main event-listener effect (which depends on
  // `showNotice`) would tear down and re-subscribe every listener on each
  // toggle. The callback stays stable for the lifetime of the component.
  const showNotice = useCallback(
    (message: string, tone: ToastNotice["tone"] = "info") => {
      if (!notificationsEnabledRef.current) {
        return;
      }

      setToast({ id: Date.now(), message, tone });
    },
    [],
  );

  const refresh = useCallback(async () => {
    setLoadError(null);
    setLoadState((current) => (current === "ready" ? current : "loading"));

    try {
      const [data, devices, modelList] = await Promise.all([
        getDashboardData(),
        listMicrophones().catch(() => null),
        listModels().catch(() => null),
      ]);
      setDashboardData(data);
      if (devices) {
        setMicrophones(devices);
      }
      if (modelList) {
        setModels(modelList);
      }
      setLoadState("ready");
    } catch (error) {
      setLoadError(commandErrorMessage(error));
      setLoadState("error");
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Seamless, branded auto-install — ON LAUNCH ONLY. Downloads the signed
  // update package quietly (NSIS installMode "quiet"; currentUser = no
  // elevation) behind the Scribe-branded UpdateOverlay, wiring real progress
  // into it, then relaunches. Guarded to run at most once per launch.
  //
  // HARD GUARDRAILS: any failure/timeout (offline, unsigned release, install
  // error) hides the overlay, logs a warning, and falls back to the normal
  // chip/manual path — it must NEVER block the app or brick updates. Runs only
  // when BOTH autoUpdateCheckEnabled AND autoInstallUpdates are on.
  const runAutoInstall = useCallback(async (version: string) => {
    setAutoUpdateState({ phase: "preparing", version });
    try {
      // The updater's own check() resolves the *signed* package (null if the
      // latest release ships no updater artifact); this is the same call the
      // manual About → Install path uses, so behavior matches exactly.
      const update = await checkUpdaterPackage();
      if (!update) {
        throw new Error("No signed update package for the latest release.");
      }
      let downloaded = 0;
      let total: number | null = null;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? null;
            setAutoUpdateState({
              phase: "downloading",
              version,
              percent: total ? 0 : null,
            });
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            setAutoUpdateState({
              phase: "downloading",
              version,
              percent: total ? (downloaded / total) * 100 : null,
            });
            break;
          case "Finished":
            setAutoUpdateState({ phase: "restarting", version });
            break;
        }
      });
      // On Windows the installer typically restarts the app itself, so we rarely
      // get here; relaunch covers the paths/platforms where execution continues.
      await relaunch();
    } catch (cause) {
      // Fall back to the normal manual path: keep the chip (updateInfo is
      // already set) and surface the branded error with a Continue button.
      console.warn("Auto-install failed; falling back to manual update.", cause);
      const message =
        cause instanceof Error ? cause.message : String(cause ?? "Unknown error");
      setAutoUpdateState({
        phase: "error",
        version,
        message: `${message} You can still install it from About → Updates.`,
      });
    }
  }, []);

  // Update check: ~5s after launch, every minute (iteration cadence), and
  // whenever the window regains focus. The first time a given version is seen we
  // fire an in-app toast AND an OS notification (the latter reaches the user even
  // when minimized to the tray). Honors the "automatically check for updates"
  // toggle; manual checks in About still work when it's off. Network failures
  // (offline, rate limit) are ignored.
  useEffect(() => {
    let lastNotifiedVersion: string | null = null;

    const runCheck = (launch: boolean) => {
      if (!autoUpdateCheckRef.current) {
        return;
      }
      void checkForUpdate()
        .then((result) => {
          // Record every successful poll so the "last checked" line in About
          // advances — this is how the polling is observable even when you're
          // already on the latest version.
          setLastUpdateCheck(Date.now());
          if (!result.updateAvailable) {
            return;
          }
          setUpdateInfo(result);
          // Notify once per newly-detected version so repeated checks don't nag.
          if (lastNotifiedVersion !== result.latestVersion) {
            lastNotifiedVersion = result.latestVersion;
            // Update availability is high-signal, so it shows regardless of the
            // in-app notifications setting.
            setToast({
              id: Date.now(),
              tone: "info",
              message: `Scribe ${result.latestVersion} is available — open About to install it.`,
            });
            void notifyUpdateAvailable(result.latestVersion);
          }
          // Seamless install only on the launch check (never mid-session — we
          // don't interrupt active work; mid-session detection keeps just the
          // chip). Guarded to fire once, and only when auto-install is on. If
          // it can't run, the chip/manual path above already covers the user.
          if (
            launch &&
            settingsLoadedRef.current &&
            autoInstallUpdatesRef.current &&
            !autoInstallAttemptedRef.current
          ) {
            autoInstallAttemptedRef.current = true;
            void runAutoInstall(result.latestVersion);
          }
        })
        .catch(() => {});
    };

    const timer = window.setTimeout(() => runCheck(true), 5000);
    // ITERATION CADENCE: poll every 60s while we're actively shipping/testing
    // updates so detection is immediate. For production this should be ~6h — the
    // on-launch and on-focus checks already make a new release feel instant, so
    // the interval is just a backstop for long open-and-idle sessions (and 60s
    // would hit GitHub's ~60/hr unauthenticated limit).
    const interval = window.setInterval(() => runCheck(false), 60 * 1000);
    const onFocus = () => runCheck(false);
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearTimeout(timer);
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, [runAutoInstall]);

  useEffect(() => {
    if (!toast) {
      return;
    }

    const timer = window.setTimeout(() => setToast(null), 4200);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    let refreshTimer: number | null = null;
    let disposed = false;
    let unlisteners: Array<() => void> = [];

    const scheduleRefresh = () => {
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
      }

      refreshTimer = window.setTimeout(() => {
        if (!disposed) {
          void refresh();
        }
      }, 250);
    };

    const setup = async () => {
      unlisteners = await Promise.all([
        listen<AppStateSnapshot>("scribe:app-state", (event) => {
          setDashboardData((current) =>
            current ? { ...current, appState: event.payload } : current,
          );
          if (event.payload.status === "Idle") {
            setLiveTranscript(null);
          }
          scheduleRefresh();
        }),
        listen<RecordingSessionInfo>("audio://recording-started", (event) => {
          setLiveTranscript(null);
          if (soundsEnabledRef.current) {
            playStartCue();
          }
          showNotice(`Recording with ${event.payload.microphoneName}.`);
        }),
        listen<PartialTranscriptEvent>(
          "scribe:partial-transcript",
          (event) => {
            // The finalized event hands off to the dictation-transcribed
            // flow, which refreshes the Last Transcript Buffer.
            setLiveTranscript(event.payload.finalized ? null : event.payload);
          },
        ),
        listen<RecordingResult>("audio://recording-stopped", (event) => {
          if (soundsEnabledRef.current) {
            playStopCue();
          }
          if (event.payload.status === "too_short") {
            showNotice(
              event.payload.reason ??
                "Recording was too short. Hold the hotkey longer and try again.",
              "error",
            );
          }
          scheduleRefresh();
        }),
        listen<RecordingErrorEvent>("audio://recording-error", (event) => {
          showNotice(event.payload.message, "error");
          scheduleRefresh();
        }),
        listen<DictationResult>("scribe:dictation-transcribed", () => {
          showNotice("Transcript ready.", "success");
          scheduleRefresh();
        }),
        listen<ModelDownloadProgress>("model://download-progress", (event) => {
          if (
            event.payload.status === "downloaded" ||
            event.payload.status === "selected"
          ) {
            showNotice("Model downloaded.", "success");
          }
          scheduleRefresh();
        }),
        listen<OutputResult>("scribe:output-completed", (event) => {
          showNotice(event.payload.message, "success");
          scheduleRefresh();
        }),
        listen<{ message: string }>("scribe:dictation-empty", (event) => {
          showNotice(event.payload.message, "info");
          scheduleRefresh();
        }),
        listen<{ message: string }>("scribe:output-failed", (event) => {
          showNotice(event.payload.message, "error");
          scheduleRefresh();
        }),
        // Voice Transform Selection outcome. The user invoked this by hotkey, so
        // its result is high-signal: toast directly (like hotkey-failure notices)
        // rather than gating on the notifications setting — a silent failure
        // would leave the selection unchanged with no explanation.
        listen<string>("scribe:selection-transformed", () => {
          setToast({
            id: Date.now(),
            tone: "success",
            message: "Selection transformed.",
          });
        }),
        listen<string>("scribe:selection-transform-failed", (event) => {
          setToast({
            id: Date.now(),
            tone: "error",
            message: event.payload || "Couldn't transform the selection.",
          });
        }),
        listen<{ route: string }>("scribe:navigate", (event) => {
          const route = routeToView(event.payload.route);
          if (route) {
            setActiveView(route);
          }
        }),
        listen<HotkeyRegistrationFailedEvent>(
          "scribe:hotkey-registration-failed",
          (event) => {
            const details = event.payload.failures
              .map(
                (failure) =>
                  `${formatHotkey(failure.shortcut)} — ${failure.message}`,
              )
              .join("; ");
            // Always surface hotkey failures, regardless of notification setting.
            setToast({
              id: Date.now(),
              tone: "error",
              message: `Hotkey(s) failed to register: ${details}`,
            });
          },
        ),
      ]);

      if (disposed) {
        unlisteners.forEach((unlisten) => unlisten());
      }
    };

    void setup();

    return () => {
      disposed = true;
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
      }
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [refresh, showNotice]);

  const persistSettings = useCallback(
    async (patch: SettingsPatch) => {
      if (!dashboardData) {
        return;
      }

      const previousSettings = dashboardData.settings;
      const nextSettings = { ...previousSettings, ...patch };

      setSaveError(null);
      setSavingSettings(true);
      setDashboardData((current) =>
        current ? { ...current, settings: nextSettings } : current,
      );

      try {
        // Persist and reconcile with the backend's canonical settings only —
        // do NOT refetch the whole dashboard here. A blanket refresh() re-renders
        // every view (and resets scroll) on each toggle; the optimistic update
        // above plus this reconcile already reflect the change. Dashboard-derived
        // data (app state, last transcript) doesn't depend on a settings toggle.
        const savedSettings = await updateSettings(nextSettings);
        setDashboardData((current) =>
          current ? { ...current, settings: savedSettings } : current,
        );
      } catch (error) {
        setDashboardData((current) =>
          current ? { ...current, settings: previousSettings } : current,
        );
        setSaveError(commandErrorMessage(error));
      } finally {
        setSavingSettings(false);
      }
    },
    [dashboardData],
  );

  const handleClearLastTranscript = useCallback(async () => {
    setClearingLastTranscript(true);
    setSaveError(null);

    try {
      await clearLastTranscript();
      await refresh();
    } catch (error) {
      setSaveError(commandErrorMessage(error));
    } finally {
      setClearingLastTranscript(false);
    }
  }, [refresh]);

  const handleOutputResult = useCallback(
    async (result: OutputResult) => {
      await refresh();
      if (result.clipboardRestoreError) {
        setSaveError(result.clipboardRestoreError);
      }
    },
    [refresh],
  );

  const handlePasteLastTranscript = useCallback(async () => {
    setPastingLastTranscript(true);
    setSaveError(null);

    try {
      const result = await pasteLastTranscript();
      await handleOutputResult(result);
    } catch (error) {
      setSaveError(commandErrorMessage(error));
      await refresh();
    } finally {
      setPastingLastTranscript(false);
    }
  }, [handleOutputResult, refresh]);

  const handleCopyLastTranscript = useCallback(async () => {
    setCopyingLastTranscript(true);
    setSaveError(null);

    try {
      const result = await copyLastTranscript();
      await handleOutputResult(result);
    } catch (error) {
      setSaveError(commandErrorMessage(error));
      await refresh();
    } finally {
      setCopyingLastTranscript(false);
    }
  }, [handleOutputResult, refresh]);

  const handleStartRecording = useCallback(async () => {
    if (!dashboardData) {
      return;
    }

    setRecordingBusy(true);
    setSaveError(null);

    try {
      await startRecording({
        microphoneId: dashboardData.settings.selectedMicId,
      });
    } catch (error) {
      setSaveError(commandErrorMessage(error));
      await refresh();
    } finally {
      setRecordingBusy(false);
    }
  }, [dashboardData, refresh]);

  const handleStopRecording = useCallback(async () => {
    setRecordingBusy(true);
    setSaveError(null);

    try {
      const recording = await stopRecording();
      if (recording.status === "completed" || recording.status === "timed_out") {
        await transcribeRecording(recording);
      } else if (recording.reason) {
        setSaveError(recording.reason);
      }
      await refresh();
    } catch (error) {
      setSaveError(commandErrorMessage(error));
      await refresh();
    } finally {
      setRecordingBusy(false);
    }
  }, [refresh]);

  const handleCancelRecording = useCallback(async () => {
    setRecordingBusy(true);
    setSaveError(null);

    try {
      await cancelRecording();
      await refresh();
    } catch (error) {
      setSaveError(commandErrorMessage(error));
      await refresh();
    } finally {
      setRecordingBusy(false);
    }
  }, [refresh]);

  const openSettings = useCallback((tabId?: string) => {
    setSettingsTabId(tabId ?? null);
    setActiveView("Settings");
  }, []);

  const actions: ViewActions = {
    cancelRecording: handleCancelRecording,
    clearLastTranscript: handleClearLastTranscript,
    clearingLastTranscript,
    copyLastTranscript: handleCopyLastTranscript,
    copyingLastTranscript,
    recordingBusy,
    openSettings,
    pasteLastTranscript: handlePasteLastTranscript,
    pastingLastTranscript,
    refresh,
    saveError,
    savingSettings,
    startRecording: handleStartRecording,
    stopRecording: handleStopRecording,
    updateSettings: (patch) => {
      void persistSettings(patch);
    },
  };

  return (
    <div className="app-shell">
      {autoUpdateState ? (
        <UpdateOverlay
          state={autoUpdateState}
          onDismiss={() => setAutoUpdateState(null)}
        />
      ) : null}
      <TitleBar isDev={isDevFlavor} />
      <div className="app-body">
      <aside className="sidebar">
        <div className="brand">
          <img className="brand-mark" src={scribeIcon} alt="" aria-hidden="true" />
          <div>
            <div className="brand-name">
              Scribe
              {isDevFlavor ? <span className="brand-badge">DEV</span> : null}
            </div>
            <div className="brand-subtitle">Private local dictation</div>
          </div>
        </div>

        <nav className="nav-list" aria-label="Primary">
          {visibleNavItems.map((item) => {
            const Icon = item.Icon;
            return (
              <button
                className={
                  item.label === activeView ? "nav-item active" : "nav-item"
                }
                key={item.label}
                onClick={() => setActiveView(item.label)}
                type="button"
              >
                <Icon aria-hidden="true" className="nav-icon" size={15} />
                {item.label}
              </button>
            );
          })}
        </nav>
      </aside>

      <main className="dashboard">
        <header className="topbar">
          <div>
            <h1>{activeView}</h1>
          </div>
          <div className="topbar-actions">
            {updateInfo?.updateAvailable ? (
              <button
                className="secondary-button update-available-button"
                onClick={() => setActiveView("About")}
                title={`Scribe v${updateInfo.latestVersion} is available — open About to install it`}
                type="button"
              >
                <Download aria-hidden="true" size={14} />
                Update v{updateInfo.latestVersion}
              </button>
            ) : null}
            <button
              className="secondary-button"
              onClick={() => setActiveView("History")}
              type="button"
            >
              <HistoryIcon aria-hidden="true" size={14} />
              History
            </button>
            {dashboardData?.appState.status === "Recording" ? (
              <>
                <button
                  className="primary-button stop-button"
                  disabled={recordingBusy}
                  onClick={() => void handleStopRecording()}
                  type="button"
                >
                  <Square aria-hidden="true" size={14} />
                  {recordingBusy ? "Working..." : "Stop"}
                </button>
                <button
                  className="ghost-button"
                  disabled={recordingBusy}
                  onClick={() => void handleCancelRecording()}
                  type="button"
                >
                  <Eraser aria-hidden="true" size={13} />
                  Discard
                </button>
              </>
            ) : (
              <button
                className="primary-button"
                disabled={recordingBusy || !canStartRecording(dashboardData)}
                onClick={() => void handleStartRecording()}
                type="button"
              >
                <Mic aria-hidden="true" size={14} />
                {recordingBusy ? "Working..." : "Start"}
              </button>
            )}
          </div>
        </header>

        {saveError ? (
          <InlineError message={saveError} onRetry={refresh} />
        ) : null}

        {loadState === "loading" && !dashboardData ? <LoadingPanel /> : null}
        {loadState === "error" && !dashboardData ? (
          <ErrorPanel message={loadError} onRetry={refresh} />
        ) : null}
        {dashboardData
          ? renderView(
              activeView,
              setActiveView,
              dashboardData,
              actions,
              microphones,
              models,
              liveTranscript,
              settingsTabId,
              showNotice,
              lastUpdateCheck,
              updateInfo,
            )
          : null}
        {toast ? <Toast notice={toast} /> : null}
      </main>
      </div>
    </div>
  );
}

function renderView(
  activeView: ViewName,
  setActiveView: (view: ViewName) => void,
  data: DashboardData,
  actions: ViewActions,
  microphones: MicrophoneInfo[] | null,
  models: ModelInfo[] | null,
  liveTranscript: PartialTranscriptEvent | null,
  settingsTabId: string | null,
  showNotice: (message: string, tone?: ToastNotice["tone"]) => void,
  lastUpdateCheck: number | null,
  updateInfo: UpdateCheckResult | null,
) {
  switch (activeView) {
    case "Transcribe":
      return (
        <TranscribeView
          actions={actions}
          data={data}
          liveTranscript={liveTranscript}
        />
      );
    case "History":
      return <HistoryView actions={actions} data={data} />;
    case "Notes":
      return <HistoryView actions={actions} data={data} notesOnly />;
    case "Sync":
      return <SyncView actions={actions} settings={data.settings} />;
    case "Stats":
      return <StatsView stats={data.stats} />;
    case "Settings":
      return (
        <SettingsView
          actions={actions}
          initialTabId={settingsTabId}
          settings={data.settings}
        />
      );
    case "Data & Privacy":
      return <DataPrivacyView actions={actions} settings={data.settings} />;
    case "Hotkeys":
      return <HotkeysView actions={actions} settings={data.settings} />;
    case "Models":
      return <ModelsView actions={actions} settings={data.settings} />;
    case "Audio":
      return <AudioView actions={actions} settings={data.settings} />;
    case "Developer":
      return (
        <DeveloperView
          actions={actions}
          refresh={actions.refresh}
          settings={data.settings}
        />
      );
    case "About":
      return (
        <AboutView
          autoCheckEnabled={data.settings.autoUpdateCheckEnabled}
          autoInstallEnabled={data.settings.autoInstallUpdates}
          autoUpdateInfo={updateInfo}
          lastUpdateCheck={lastUpdateCheck}
          onToggleAutoCheck={(enabled) =>
            actions.updateSettings({ autoUpdateCheckEnabled: enabled })
          }
          onToggleAutoInstall={(enabled) =>
            actions.updateSettings({ autoInstallUpdates: enabled })
          }
          setActiveView={setActiveView}
        />
      );
    case "Dashboard":
    default:
      return (
        <DashboardView
          actions={actions}
          data={data}
          microphones={microphones}
          models={models}
          setActiveView={setActiveView}
          showNotice={showNotice}
        />
      );
  }
}

export default App;
