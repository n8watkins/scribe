import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getName as getAppName } from "@tauri-apps/api/app";
import {
  BarChart3,
  Database,
  Eraser,
  Gauge,
  History as HistoryIcon,
  Info,
  Keyboard,
  Mic,
  NotebookPen,
  Radio,
  Settings as SettingsIcon,
  ShieldCheck,
  Square,
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

const navItems: { label: ViewName; Icon: LucideIcon }[] = [
  { label: "Dashboard", Icon: Gauge },
  { label: "Transcribe", Icon: Mic },
  { label: "History", Icon: HistoryIcon },
  { label: "Notes", Icon: NotebookPen },
  { label: "Stats", Icon: BarChart3 },
  { label: "Settings", Icon: SettingsIcon },
  { label: "Data & Privacy", Icon: ShieldCheck },
  { label: "Hotkeys", Icon: Keyboard },
  { label: "Models", Icon: Database },
  { label: "Audio", Icon: Radio },
  { label: "About", Icon: Info },
];

const viewTitles: Record<ViewName, { eyebrow: string; title: string }> = {
  Dashboard: {
    eyebrow: "Dashboard",
    title: "Local speech-to-text control center",
  },
  Transcribe: {
    eyebrow: "Transcribe",
    title: "Record, review, and route the next transcript",
  },
  History: {
    eyebrow: "History",
    title: "Search and reuse local transcripts",
  },
  Notes: {
    eyebrow: "Notes",
    title: "Dictated notes (hold ~ and tap Q)",
  },
  Stats: {
    eyebrow: "Stats",
    title: "Local dictation usage at a glance",
  },
  Settings: {
    eyebrow: "Settings",
    title: "Output and app behavior",
  },
  "Data & Privacy": {
    eyebrow: "Data & Privacy",
    title: "History retention and local data",
  },
  Hotkeys: {
    eyebrow: "Hotkeys",
    title: "Global shortcuts and recording controls",
  },
  Models: {
    eyebrow: "Models",
    title: "Local Whisper model manager",
  },
  Audio: {
    eyebrow: "Audio",
    title: "Microphone input and recording quality",
  },
  About: {
    eyebrow: "About",
    title: "Private local dictation for Windows",
  },
};

function App() {
  const [activeView, setActiveView] = useState<ViewName>("Dashboard");
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

  // The dev flavor labels itself so two running instances are tellable apart.
  useEffect(() => {
    void getAppName()
      .then((name) => setIsDevFlavor(name.includes("Dev")))
      .catch(() => {});
  }, []);
  const [liveTranscript, setLiveTranscript] =
    useState<PartialTranscriptEvent | null>(null);
  const soundsEnabledRef = useRef(false);
  const heading = viewTitles[activeView];

  useEffect(() => {
    soundsEnabledRef.current = dashboardData?.settings.soundsEnabled ?? false;
  }, [dashboardData?.settings.soundsEnabled]);

  const showNotice = useCallback(
    (message: string, tone: ToastNotice["tone"] = "info") => {
      if (!dashboardData?.settings.notificationsEnabled) {
        return;
      }

      setToast({ id: Date.now(), message, tone });
    },
    [dashboardData?.settings.notificationsEnabled],
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

  // One quiet update check shortly after launch; network failures (offline,
  // private repo) are ignored.
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void checkForUpdate()
        .then((result) => {
          if (result.updateAvailable) {
            setToast({
              id: Date.now(),
              tone: "info",
              message: `Scribe v${result.latestVersion} is available — see About to view the release.`,
            });
          }
        })
        .catch(() => {});
    }, 5000);
    return () => window.clearTimeout(timer);
  }, []);

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
        const savedSettings = await updateSettings(nextSettings);
        setDashboardData((current) =>
          current ? { ...current, settings: savedSettings } : current,
        );
        await refresh();
      } catch (error) {
        setDashboardData((current) =>
          current ? { ...current, settings: previousSettings } : current,
        );
        setSaveError(commandErrorMessage(error));
      } finally {
        setSavingSettings(false);
      }
    },
    [dashboardData, refresh],
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

  const actions: ViewActions = {
    cancelRecording: handleCancelRecording,
    clearLastTranscript: handleClearLastTranscript,
    clearingLastTranscript,
    copyLastTranscript: handleCopyLastTranscript,
    copyingLastTranscript,
    recordingBusy,
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
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">S</div>
          <div>
            <div className="brand-name">
              Scribe
              {isDevFlavor ? <span className="brand-badge">DEV</span> : null}
            </div>
            <div className="brand-subtitle">Private local dictation</div>
          </div>
        </div>

        <nav className="nav-list" aria-label="Primary">
          {navItems.map((item) => {
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

        <div className="privacy-panel">
          <div className="privacy-status">
            <ShieldCheck aria-hidden="true" size={14} />
            Offline ready
          </div>
          <p>Audio and transcripts stay on this device.</p>
        </div>
      </aside>

      <main className="dashboard">
        <header className="topbar">
          <div>
            <p className="eyebrow">{heading.eyebrow}</p>
            <h1>{heading.title}</h1>
          </div>
          <div className="topbar-actions">
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
                  Cancel
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
            )
          : null}
        {toast ? <Toast notice={toast} /> : null}
      </main>
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
    case "Stats":
      return <StatsView stats={data.stats} />;
    case "Settings":
      return <SettingsView actions={actions} settings={data.settings} />;
    case "Data & Privacy":
      return <DataPrivacyView actions={actions} settings={data.settings} />;
    case "Hotkeys":
      return <HotkeysView actions={actions} settings={data.settings} />;
    case "Models":
      return <ModelsView actions={actions} settings={data.settings} />;
    case "Audio":
      return <AudioView actions={actions} settings={data.settings} />;
    case "About":
      return <AboutView />;
    case "Dashboard":
    default:
      return (
        <DashboardView
          actions={actions}
          data={data}
          microphones={microphones}
          models={models}
          setActiveView={setActiveView}
        />
      );
  }
}

export default App;
