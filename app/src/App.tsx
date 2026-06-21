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
import { detectUpdate } from "./lib/updates";
import { deriveCustomThemeVars } from "./lib/customTheme";
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
  Palette,
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
import { ThemesView } from "./views/Themes";
import { AboutView } from "./views/About";
import { DeveloperView } from "./views/Developer";
import { SyncView } from "./views/Sync";
import scribeIcon from "./assets/scribe-icon.png";

// Dummy colors used only to enumerate the `--scribe-*` keys the custom theme
// owns, so the theme effect can `removeProperty` exactly those when switching
// away from custom (the values are irrelevant — only the key set is read).
const EMPTY_CUSTOM_THEME = {
  background: "#000000",
  surface: "#000000",
  accent: "#000000",
  text: "#000000",
  textMuted: "#000000",
} as const;

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
  { label: "Themes", Icon: Palette },
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

// Toast stack limits: at most MAX_TOASTS visible at once — so a later notice
// never instantly clobbers an earlier one the user hasn't read (e.g. the
// mic-disconnect notice immediately followed by the stop notice) — each shown
// for TOAST_DURATION_MS before it auto-dismisses.
const MAX_TOASTS = 2;
const TOAST_DURATION_MS = 5000;

// When the mic is unplugged mid-recording the backend salvages + transcribes
// what it captured, which would fire the normal "Transcript ready." and paste
// notices right behind the disconnect notice — three toasts for one event. For
// this window after a disconnect, the routine success/empty notices are
// suppressed so only the single, explanatory disconnect toast shows.
const DISCONNECT_NOTICE_SUPPRESS_MS = 8000;

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
  const [toasts, setToasts] = useState<ToastNotice[]>([]);
  const toastIdRef = useRef(0);
  // Epoch (ms) until which routine dictation notices are suppressed because a
  // mic-disconnect just explained the situation in one toast. 0 = not active.
  const suppressRoutineNoticesUntilRef = useRef(0);
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
  // Version found by the on-LAUNCH check that's awaiting a silent auto-install.
  // Set ONLY by the launch check (never mid-session), so auto-install stays
  // launch-only even though it may actually fire from the settings-loaded effect.
  const launchInstallVersionRef = useRef<string | null>(null);

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

  // Apply the selected color theme to the document root. The CSS palette is keyed
  // off `[data-theme]` (see App.css). Default to "midnight" — which equals the
  // historical look — so there's no flash before settings load and an unknown or
  // blank stored value still renders the standard theme.
  //
  // The "custom" theme has no static CSS block: its palette is derived from the
  // user's three core colors (see deriveCustomThemeVars) and written as inline
  // `--scribe-*` properties on the root, which override the [data-theme] block.
  // Switching away from custom removes those inline props so the chosen preset's
  // block applies cleanly.
  const customTheme = dashboardData?.settings.customTheme;
  useEffect(() => {
    const theme = dashboardData?.settings.theme || "midnight";
    const root = document.documentElement;
    root.setAttribute("data-theme", theme);

    const style = root.style;
    if (theme === "custom" && customTheme) {
      const vars = deriveCustomThemeVars(customTheme);
      for (const [name, value] of Object.entries(vars)) {
        style.setProperty(name, value);
      }
    } else {
      // Not custom: strip any inline overrides a prior custom selection left, so
      // the preset's [data-theme] block is what actually paints. Removing keys
      // that were never set is a harmless no-op.
      for (const name of Object.keys(deriveCustomThemeVars(EMPTY_CUSTOM_THEME))) {
        style.removeProperty(name);
      }
    }

    // Cache it so the inline bootstrap in index.html can set the theme before
    // first paint on the next launch (no midnight flash for non-default themes).
    try {
      localStorage.setItem("scribe.theme", theme);
    } catch {
      // localStorage may be unavailable; the bootstrap just falls back to midnight.
    }
  }, [dashboardData?.settings.theme, customTheme]);

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
  // Append a toast to the stack and schedule its own dismissal. The stack is
  // capped at MAX_TOASTS (oldest drops out), so two notices fired back-to-back
  // both stay readable instead of the second replacing the first. Stable
  // (no deps) so it doesn't re-subscribe the event listeners below.
  const pushToast = useCallback((notice: Omit<ToastNotice, "id">) => {
    const id = (toastIdRef.current += 1);
    setToasts((prev) => [...prev, { ...notice, id }].slice(-MAX_TOASTS));
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((existing) => existing.id !== id));
    }, TOAST_DURATION_MS);
  }, []);

  const showNotice = useCallback(
    (message: string, tone: ToastNotice["tone"] = "info") => {
      if (!notificationsEnabledRef.current) {
        return;
      }

      pushToast({ message, tone });
    },
    [pushToast],
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
    // Inactivity watchdog: if the download produces no event for STALL_MS, treat
    // it as stalled so the overlay can't hang forever with no escape (review m2).
    const STALL_MS = 90_000;
    let lastActivity = Date.now();
    let stallTimer: number | undefined;
    try {
      // The updater's own check() resolves the *signed* package — the same call
      // the manual About → Install path uses. null means no updater artifact is
      // published for the latest release yet (e.g. the GitHub tag exists but
      // latest.json hasn't landed) OR the updater considers us current. On the
      // silent launch path that's NOT an error: just dismiss and keep the chip —
      // don't pop a scary error screen on launch (review m1).
      const update = await checkUpdaterPackage();
      if (!update) {
        console.warn("Auto-install: no signed update package yet; skipping silently.");
        setAutoUpdateState(null);
        return;
      }
      let downloaded = 0;
      let total: number | null = null;
      const stalled = new Promise<never>((_, reject) => {
        const tick = () => {
          if (Date.now() - lastActivity > STALL_MS) {
            reject(
              new Error("The update download stalled — check your connection."),
            );
          } else {
            stallTimer = window.setTimeout(tick, 5000);
          }
        };
        stallTimer = window.setTimeout(tick, 5000);
      });
      await Promise.race([
        update.downloadAndInstall((event) => {
          lastActivity = Date.now();
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
        }),
        stalled,
      ]);
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
    } finally {
      if (stallTimer) {
        window.clearTimeout(stallTimer);
      }
    }
  }, []);

  // Fire the on-launch auto-install once everything required is true: a LAUNCH
  // check found an update, the user's settings have actually loaded (so a saved
  // opt-out is honored, not raced), both update toggles are on, and we haven't
  // attempted it. Called right after the launch check AND from the
  // settings-loaded effect below — whichever resolves last triggers it, so a slow
  // startup DB read no longer silently skips the install (review M1).
  const maybeRunLaunchInstall = useCallback(() => {
    if (
      launchInstallVersionRef.current &&
      settingsLoadedRef.current &&
      autoUpdateCheckRef.current &&
      autoInstallUpdatesRef.current &&
      !autoInstallAttemptedRef.current
    ) {
      autoInstallAttemptedRef.current = true;
      void runAutoInstall(launchInstallVersionRef.current);
    }
  }, [runAutoInstall]);

  // If the launch check found an update before settings finished loading, install
  // once they do. No-op after the one-shot guard trips, so re-running on every
  // dashboardData change is harmless (review M1).
  useEffect(() => {
    maybeRunLaunchInstall();
  }, [maybeRunLaunchInstall, dashboardData]);

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
      void detectUpdate()
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
            pushToast({
              tone: "info",
              message: `Scribe ${result.latestVersion} is available — open About to install it.`,
            });
            void notifyUpdateAvailable(result.latestVersion);
          }
          // Seamless install only on the launch check (never mid-session — we
          // don't interrupt active work; mid-session detection keeps just the
          // chip). Record the launch-detected version and try; if settings
          // haven't loaded yet, the settings-loaded effect fires it once they do,
          // so it isn't silently skipped on a slow startup (review M1).
          if (launch) {
            launchInstallVersionRef.current = result.latestVersion;
            maybeRunLaunchInstall();
          }
        })
        .catch(() => {});
    };

    const timer = window.setTimeout(() => runCheck(true), 5000);
    // Poll every 60s while testing so detection is immediate. Detection now uses
    // the updater's latest.json (a release CDN file) rather than the GitHub REST
    // API, so this no longer trips the API's ~60/hr rate limit (which a 1-min
    // poll did, returning 403 to every check). For production ~6h is plenty —
    // the on-launch and on-focus checks already make a new release feel instant.
    const interval = window.setInterval(() => runCheck(false), 60 * 1000);
    const onFocus = () => runCheck(false);
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearTimeout(timer);
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, [maybeRunLaunchInstall]);

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
          // A fresh recording ends any prior disconnect episode, so its own
          // notices are never suppressed by a stale flag.
          suppressRoutineNoticesUntilRef.current = 0;
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
          // A mid-recording disconnect is followed by salvage transcription +
          // paste, whose routine notices would bury this one. Suppress those for
          // a short window so the disconnect speaks for itself.
          if (event.payload.code === "microphone_unavailable") {
            suppressRoutineNoticesUntilRef.current =
              Date.now() + DISCONNECT_NOTICE_SUPPRESS_MS;
          }
          showNotice(event.payload.message, "error");
          scheduleRefresh();
        }),
        listen<DictationResult>("scribe:dictation-transcribed", () => {
          if (Date.now() >= suppressRoutineNoticesUntilRef.current) {
            showNotice("Transcript ready.", "success");
          }
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
          if (Date.now() >= suppressRoutineNoticesUntilRef.current) {
            showNotice(event.payload.message, "success");
          }
          scheduleRefresh();
        }),
        listen<{ message: string }>("scribe:dictation-empty", (event) => {
          if (Date.now() >= suppressRoutineNoticesUntilRef.current) {
            showNotice(event.payload.message, "info");
          }
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
          pushToast({
            tone: "success",
            message: "Selection transformed.",
          });
        }),
        listen<string>("scribe:selection-transform-failed", (event) => {
          pushToast({
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
            pushToast({
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
  }, [refresh, showNotice, pushToast]);

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
        {toasts.length > 0 ? (
          <div className="toast-stack">
            {toasts.map((notice) => (
              <Toast key={notice.id} notice={notice} />
            ))}
          </div>
        ) : null}
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
    case "Themes":
      return <ThemesView actions={actions} settings={data.settings} />;
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
