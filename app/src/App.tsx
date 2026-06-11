import { useCallback, useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  Archive,
  CheckCircle2,
  Clipboard,
  ClipboardPaste,
  Copy,
  Database,
  Download,
  Eraser,
  FolderOpen,
  Gauge,
  History as HistoryIcon,
  Info,
  Keyboard,
  Mic,
  MonitorCog,
  Pencil,
  Play,
  Radio,
  RefreshCw,
  Search,
  Settings as SettingsIcon,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  Trash2,
  type LucideIcon,
} from "lucide-react";
import {
  clearLastTranscript,
  clearTranscriptHistory,
  commandErrorMessage,
  cancelModelDownload,
  cancelRecording,
  copyLastTranscript,
  copyTranscript,
  deleteTranscript,
  deleteModel,
  downloadModel,
  getDashboardData,
  listMicrophones,
  listModels,
  pasteLastTranscript,
  pasteTranscript,
  recordTestClip,
  retryModelDownload,
  searchTranscripts,
  selectModel,
  startRecording,
  stopRecording,
  transcribeRecording,
  updateTranscript,
  updateSettings,
  type AudioLevelEvent,
  type AppSettings,
  type AppStateSnapshot,
  type BasicStats,
  type DashboardData,
  type DictationResult,
  type HistoryRetentionDays,
  type MicrophoneInfo,
  type ModelDownloadProgress,
  type ModelInfo,
  type OutputMode,
  type OutputResult,
  type PasteMethod,
  type RecordingMode,
  type RecordingResult,
  type RecordingSessionInfo,
  type RecordingErrorEvent,
  type Transcript,
} from "./backend";
import "./App.css";

type ViewName =
  | "Dashboard"
  | "Transcribe"
  | "History"
  | "Settings"
  | "Hotkeys"
  | "Models"
  | "Audio"
  | "About";

type LoadState = "loading" | "ready" | "error";

type SettingsPatch = Partial<AppSettings>;

type ViewActions = {
  cancelRecording: () => Promise<void>;
  clearLastTranscript: () => Promise<void>;
  clearingLastTranscript: boolean;
  copyLastTranscript: () => Promise<void>;
  copyingLastTranscript: boolean;
  recordingBusy: boolean;
  pasteLastTranscript: () => Promise<void>;
  pastingLastTranscript: boolean;
  refresh: () => Promise<void>;
  saveError: string | null;
  savingSettings: boolean;
  startRecording: () => Promise<void>;
  stopRecording: () => Promise<void>;
  updateSettings: (patch: SettingsPatch) => void;
};

const navItems: { label: ViewName; Icon: LucideIcon }[] = [
  { label: "Dashboard", Icon: Gauge },
  { label: "Transcribe", Icon: Mic },
  { label: "History", Icon: HistoryIcon },
  { label: "Settings", Icon: SettingsIcon },
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
  Settings: {
    eyebrow: "Settings",
    title: "Privacy, output, and app behavior",
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

const outputModeOptions: { label: string; value: OutputMode }[] = [
  { label: "Save Only", value: "save_only" },
  { label: "Auto Paste", value: "auto_paste" },
  { label: "Copy", value: "copy_clipboard" },
  { label: "Copy + Paste", value: "copy_and_paste" },
];

const pasteMethodOptions: { label: string; value: PasteMethod }[] = [
  { label: "Direct Insert", value: "direct_insert" },
  { label: "Compatibility Paste", value: "clipboard_restore" },
];

const recordingModeOptions: { label: string; value: RecordingMode }[] = [
  { label: "Hold", value: "hold" },
  { label: "Toggle", value: "toggle" },
  { label: "Both", value: "both" },
];

type ToastNotice = {
  id: number;
  tone: "info" | "success" | "error";
  message: string;
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
  const [toast, setToast] = useState<ToastNotice | null>(null);
  const heading = viewTitles[activeView];

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
      const data = await getDashboardData();
      setDashboardData(data);
      setLoadState("ready");
    } catch (error) {
      setLoadError(commandErrorMessage(error));
      setLoadState("error");
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

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
        listen<AppStateSnapshot>("localdictate:app-state", (event) => {
          setDashboardData((current) =>
            current ? { ...current, appState: event.payload } : current,
          );
          scheduleRefresh();
        }),
        listen<RecordingSessionInfo>("audio://recording-started", (event) => {
          showNotice(`Recording with ${event.payload.microphoneName}.`);
        }),
        listen<RecordingResult>("audio://recording-stopped", (event) => {
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
        listen<DictationResult>("localdictate:dictation-transcribed", () => {
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
        listen<OutputResult>("localdictate:output-completed", (event) => {
          showNotice(event.payload.message, "success");
          scheduleRefresh();
        }),
        listen<{ message: string }>("localdictate:output-failed", (event) => {
          showNotice(event.payload.message, "error");
          scheduleRefresh();
        }),
        listen<{ route: string }>("localdictate:navigate", (event) => {
          const route = routeToView(event.payload.route);
          if (route) {
            setActiveView(route);
          }
        }),
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
          <div className="brand-mark">LD</div>
          <div>
            <div className="brand-name">LocalDictate</div>
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
                <Icon aria-hidden="true" className="nav-icon" size={17} />
                {item.label}
              </button>
            );
          })}
        </nav>

        <div className="privacy-panel">
          <div className="privacy-status">
            <ShieldCheck aria-hidden="true" size={16} />
            Offline ready
          </div>
          <p>Audio and transcripts stay on this device after model download.</p>
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
              <HistoryIcon aria-hidden="true" size={16} />
              Open history
            </button>
            <button
              className="primary-button"
              disabled={recordingBusy || !canStartRecording(dashboardData)}
              onClick={() => void handleStartRecording()}
              type="button"
            >
              <Mic aria-hidden="true" size={16} />
              {recordingBusy ? "Working..." : "Start"}
            </button>
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
          ? renderView(activeView, setActiveView, dashboardData, actions)
          : null}
        {dashboardData?.settings.showFloatingPill ? (
          <FloatingPill
            appState={dashboardData.appState}
            outputMode={dashboardData.settings.outputMode}
            pasteHotkey={dashboardData.settings.hotkeys.pasteLastTranscript}
          />
        ) : null}
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
) {
  switch (activeView) {
    case "Transcribe":
      return <TranscribeView actions={actions} data={data} />;
    case "History":
      return <HistoryView actions={actions} data={data} />;
    case "Settings":
      return <SettingsView actions={actions} settings={data.settings} />;
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
        <DashboardView actions={actions} data={data} setActiveView={setActiveView} />
      );
  }
}

function DashboardView({
  actions,
  data,
  setActiveView,
}: {
  actions: ViewActions;
  data: DashboardData;
  setActiveView: (view: ViewName) => void;
}) {
  const { appState, lastTranscript, recentTranscripts, settings, stats } = data;

  return (
    <>
      <section className="status-grid" aria-label="Current setup">
        <StatusCard
          action="Record"
          Icon={Gauge}
          label="Current status"
          onAction={() => setActiveView("Transcribe")}
          status={<StatePill appState={appState} />}
          value={appState.status}
        />
        <StatusCard
          action="Choose"
          Icon={Mic}
          label="Active microphone"
          onAction={() => setActiveView("Audio")}
          status={<span className="status-dot success" />}
          value={settings.selectedMicId ?? "Default communications device"}
        />
        <StatusCard
          action="Manage"
          Icon={Database}
          label="Active model"
          onAction={() => setActiveView("Models")}
          status={
            settings.selectedModelId ? (
              <span className="pill selected">Selected</span>
            ) : (
              <span className="pill pending">Not selected</span>
            )
          }
          value={settings.selectedModelId ?? "Choose a model"}
        />
        <StatusCard
          action="Change"
          Icon={Clipboard}
          label="Output mode"
          onAction={() => setActiveView("Settings")}
          status={<span className="pill preserve">{clipboardStatus(settings)}</span>}
          value={outputModeLabel(settings.outputMode)}
        />
      </section>

      <section className="main-grid">
        <LastTranscriptCard
          clearing={actions.clearingLastTranscript}
          copying={actions.copyingLastTranscript}
          onClear={actions.clearLastTranscript}
          onCopy={actions.copyLastTranscript}
          onPaste={actions.pasteLastTranscript}
          pasting={actions.pastingLastTranscript}
          transcript={lastTranscript}
        />

        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Hotkeys</h2>
            <button
              className="ghost-button"
              onClick={() => setActiveView("Hotkeys")}
              type="button"
            >
              <Keyboard aria-hidden="true" size={15} />
              Rebind
            </button>
          </div>
          <HotkeyList compact settings={settings} />
        </article>

        <RecentTranscriptsCard
          recentTranscripts={recentTranscripts}
          setActiveView={setActiveView}
        />

        <StatsCard stats={stats} />
      </section>
    </>
  );
}

function TranscribeView({
  actions,
  data,
}: {
  actions: ViewActions;
  data: DashboardData;
}) {
  const { appState, lastTranscript, settings } = data;

  return (
    <section className="split-grid">
      <article className="buffer-card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Recording</p>
            <h2>Push-to-talk capture</h2>
          </div>
          <StatePill appState={appState} />
        </div>

        <div className="recording-stage">
          <Waveform />
          <div>
            <strong>{recordingStageTitle(appState.status)}</strong>
            <p className="muted">
              Hold {formatHotkey(settings.hotkeys.holdToTalk)} or use toggle
              mode. Captures are normalized to 16 kHz mono WAV before local
              transcription.
            </p>
          </div>
        </div>

        <div className="button-row">
          <button
            className="primary-button"
            disabled={actions.recordingBusy || !canStartRecording(data)}
            onClick={() => void actions.startRecording()}
            type="button"
          >
            <Mic aria-hidden="true" size={16} />
            {actions.recordingBusy ? "Working..." : "Start"}
          </button>
          <button
            className="secondary-button"
            disabled={actions.recordingBusy || appState.status !== "Recording"}
            onClick={() => void actions.stopRecording()}
            type="button"
          >
            <Square aria-hidden="true" size={15} />
            Stop
          </button>
          <button
            className="ghost-button"
            disabled={actions.recordingBusy || appState.status !== "Recording"}
            onClick={() => void actions.cancelRecording()}
            type="button"
          >
            <Eraser aria-hidden="true" size={15} />
            Cancel
          </button>
        </div>
      </article>

      <div className="stack">
        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Output behavior</h2>
            <span className="pill preserve">{clipboardStatus(settings)}</span>
          </div>
          <SegmentedControl
            disabled={actions.savingSettings}
            onChange={(outputMode) => actions.updateSettings({ outputMode })}
            options={outputModeOptions}
            selected={settings.outputMode}
          />
        </article>

        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Paste method</h2>
          </div>
          <SegmentedControl
            disabled={actions.savingSettings}
            onChange={(pasteMethod) => actions.updateSettings({ pasteMethod })}
            options={pasteMethodOptions}
            selected={settings.pasteMethod}
          />
        </article>

        <LastTranscriptCard
          clearing={actions.clearingLastTranscript}
          compact
          copying={actions.copyingLastTranscript}
          onClear={actions.clearLastTranscript}
          onCopy={actions.copyLastTranscript}
          onPaste={actions.pasteLastTranscript}
          pasting={actions.pastingLastTranscript}
          transcript={lastTranscript}
        />
      </div>
    </section>
  );
}

function HistoryView({
  actions,
  data,
}: {
  actions: ViewActions;
  data: DashboardData;
}) {
  const [query, setQuery] = useState("");
  const [offset, setOffset] = useState(0);
  const [transcripts, setTranscripts] = useState<Transcript[]>([]);
  const [total, setTotal] = useState(0);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [busyTranscriptId, setBusyTranscriptId] = useState<string | null>(null);
  const [clearingHistory, setClearingHistory] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");
  const { settings } = data;
  const pageSize = 10;

  const loadHistory = useCallback(
    async (nextOffset: number) => {
      setHistoryLoading(true);
      setHistoryError(null);

      try {
        let result = await searchTranscripts({
          query: query.trim() || undefined,
          limit: pageSize,
          offset: nextOffset,
        });
        if (
          result.total > 0 &&
          result.transcripts.length === 0 &&
          nextOffset > 0
        ) {
          result = await searchTranscripts({
            query: query.trim() || undefined,
            limit: pageSize,
            offset: Math.max(0, nextOffset - pageSize),
          });
        }

        setTranscripts(result.transcripts);
        setTotal(result.total);
        setOffset(result.offset);
      } catch (error) {
        setHistoryError(commandErrorMessage(error));
      } finally {
        setHistoryLoading(false);
      }
    },
    [query],
  );

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void loadHistory(0);
    }, 180);

    return () => window.clearTimeout(timer);
  }, [loadHistory]);

  useEffect(() => {
    void loadHistory(offset);
  }, [data.lastTranscript?.id, data.stats.dictationsToday, loadHistory, offset]);

  const refreshAfterMutation = useCallback(async () => {
    await actions.refresh();
    await loadHistory(offset);
  }, [actions, loadHistory, offset]);

  const handlePasteTranscript = useCallback(
    async (id: string) => {
      setBusyTranscriptId(id);
      setHistoryError(null);

      try {
        const result = await pasteTranscript(id);
        if (result.clipboardRestoreError) {
          setHistoryError(result.clipboardRestoreError);
        }
        await refreshAfterMutation();
      } catch (error) {
        setHistoryError(commandErrorMessage(error));
      } finally {
        setBusyTranscriptId(null);
      }
    },
    [refreshAfterMutation],
  );

  const handleCopyTranscript = useCallback(
    async (id: string) => {
      setBusyTranscriptId(id);
      setHistoryError(null);

      try {
        await copyTranscript(id);
        await refreshAfterMutation();
      } catch (error) {
        setHistoryError(commandErrorMessage(error));
      } finally {
        setBusyTranscriptId(null);
      }
    },
    [refreshAfterMutation],
  );

  const startEdit = useCallback((transcript: Transcript) => {
    setEditingId(transcript.id);
    setEditText(transcript.text);
  }, []);

  const cancelEdit = useCallback(() => {
    setEditingId(null);
    setEditText("");
  }, []);

  const saveEdit = useCallback(
    async (id: string) => {
      setBusyTranscriptId(id);
      setHistoryError(null);

      try {
        await updateTranscript(id, editText);
        cancelEdit();
        await refreshAfterMutation();
      } catch (error) {
        setHistoryError(commandErrorMessage(error));
      } finally {
        setBusyTranscriptId(null);
      }
    },
    [cancelEdit, editText, refreshAfterMutation],
  );

  const handleDeleteTranscript = useCallback(
    async (id: string) => {
      if (!window.confirm("Delete this transcript from local history?")) {
        return;
      }

      setBusyTranscriptId(id);
      setHistoryError(null);

      try {
        await deleteTranscript(id);
        await refreshAfterMutation();
      } catch (error) {
        setHistoryError(commandErrorMessage(error));
      } finally {
        setBusyTranscriptId(null);
      }
    },
    [refreshAfterMutation],
  );

  const handleClearHistory = useCallback(async () => {
    if (!window.confirm("Clear all saved transcript history?")) {
      return;
    }

    setClearingHistory(true);
    setHistoryError(null);

    try {
      await clearTranscriptHistory();
      setOffset(0);
      await refreshAfterMutation();
    } catch (error) {
      setHistoryError(commandErrorMessage(error));
    } finally {
      setClearingHistory(false);
    }
  }, [refreshAfterMutation]);

  const pageStart = total === 0 ? 0 : offset + 1;
  const pageEnd = Math.min(offset + pageSize, total);
  const hasPrevious = offset > 0;
  const hasNext = offset + pageSize < total;

  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="toolbar-row">
          <div className="search-field">
            <Search aria-hidden="true" size={16} />
            <input
              aria-label="Search transcripts"
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Search transcripts"
              value={query}
            />
          </div>
          <select
            aria-label="Retention"
            disabled={actions.savingSettings}
            onChange={(event) =>
              actions.updateSettings({
                historyRetentionDays: retentionFromValue(
                  event.currentTarget.value,
                ),
              })
            }
            value={retentionToValue(settings.historyRetentionDays)}
          >
            <option value="7">7 day retention</option>
            <option value="30">30 day retention</option>
            <option value="90">90 day retention</option>
            <option value="365">365 day retention</option>
            <option value="forever">Forever</option>
          </select>
          <button
            className="secondary-button"
            disabled={clearingHistory || total === 0}
            onClick={() => void handleClearHistory()}
            type="button"
          >
            <Trash2 aria-hidden="true" size={15} />
            {clearingHistory ? "Clearing..." : "Clear all"}
          </button>
        </div>
      </article>

      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Transcript archive</h2>
          <Archive aria-hidden="true" size={16} />
          <span className="muted">
            {pageStart}-{pageEnd} of {total} local records
          </span>
        </div>
        {historyError ? (
          <InlineError message={historyError} onRetry={() => loadHistory(offset)} />
        ) : null}
        {!settings.historyEnabled ? (
          <EmptyState message="History is disabled. Existing records remain available until you delete them." />
        ) : null}
        {historyLoading ? (
          <div className="pending-panel">
            <RefreshCw aria-hidden="true" size={16} />
            <span>Loading transcript history...</span>
          </div>
        ) : null}
        {!historyLoading && transcripts.length === 0 ? (
          <EmptyState message="No local transcript records match this view yet." />
        ) : null}
        {!historyLoading && transcripts.length > 0 ? (
          <div className="transcript-list">
            {transcripts.map((item) => (
              <TranscriptRow
                busy={busyTranscriptId === item.id}
                editText={editingId === item.id ? editText : undefined}
                item={item}
                key={item.id}
                onCancelEdit={cancelEdit}
                onCopy={handleCopyTranscript}
                onDelete={handleDeleteTranscript}
                onEdit={startEdit}
                onEditTextChange={setEditText}
                onPaste={handlePasteTranscript}
                onSaveEdit={saveEdit}
                variant="full"
              />
            ))}
          </div>
        ) : null}
        <div className="pagination-row">
          <button
            className="secondary-button"
            disabled={!hasPrevious || historyLoading}
            onClick={() => void loadHistory(Math.max(0, offset - pageSize))}
            type="button"
          >
            Previous
          </button>
          <button
            className="secondary-button"
            disabled={!hasNext || historyLoading}
            onClick={() => void loadHistory(offset + pageSize)}
            type="button"
          >
            Next
          </button>
        </div>
      </article>
    </section>
  );
}

function SettingsView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  return (
    <section className="view-grid">
      <SectionPanel
        icon={<ShieldCheck aria-hidden="true" size={16} />}
        title="Privacy defaults"
      >
        <SettingRow
          description="Keep searchable local transcript records."
          label="History enabled"
        >
          <Toggle
            checked={settings.historyEnabled}
            disabled={actions.savingSettings}
            label="History enabled"
            onChange={(historyEnabled) => actions.updateSettings({ historyEnabled })}
          />
        </SettingRow>
        <SettingRow
          description="Store source clips beside transcript metadata."
          label="Save raw audio clips"
        >
          <Toggle
            checked={settings.saveAudioClips}
            disabled={actions.savingSettings}
            label="Save raw audio clips"
            onChange={(saveAudioClips) => actions.updateSettings({ saveAudioClips })}
          />
        </SettingRow>
        <SettingRow
          description="Automatically delete old history."
          label="Retention"
        >
          <select
            disabled={actions.savingSettings}
            onChange={(event) =>
              actions.updateSettings({
                historyRetentionDays: retentionFromValue(
                  event.currentTarget.value,
                ),
              })
            }
            value={retentionToValue(settings.historyRetentionDays)}
          >
            <option value="7">7 days</option>
            <option value="30">30 days</option>
            <option value="90">90 days</option>
            <option value="365">365 days</option>
            <option value="forever">Forever</option>
          </select>
        </SettingRow>
        <SettingRow
          description="Speech recognition language preference."
          label="Language"
        >
          <select
            disabled={actions.savingSettings}
            onChange={(event) =>
              actions.updateSettings({
                language: event.currentTarget.value === "auto" ? "auto" : "en",
              })
            }
            value={settings.language}
          >
            <option value="auto">Auto detect</option>
            <option value="en">English</option>
          </select>
        </SettingRow>
      </SectionPanel>

      <SectionPanel
        icon={<MonitorCog aria-hidden="true" size={16} />}
        title="App behavior"
      >
        <SettingRow
          description="Start LocalDictate when Windows starts."
          label="Launch at startup"
        >
          <Toggle
            checked={settings.launchAtStartup}
            disabled={actions.savingSettings}
            label="Launch at startup"
            onChange={(launchAtStartup) =>
              actions.updateSettings({ launchAtStartup })
            }
          />
        </SettingRow>
        <SettingRow
          description="Keep the app available from the system tray."
          label="Minimize to tray"
        >
          <Toggle
            checked={settings.minimizeToTray}
            disabled={actions.savingSettings}
            label="Minimize to tray"
            onChange={(minimizeToTray) =>
              actions.updateSettings({ minimizeToTray })
            }
          />
        </SettingRow>
        <SettingRow
          description="Show capture state near the cursor."
          label="Show floating pill"
        >
          <Toggle
            checked={settings.showFloatingPill}
            disabled={actions.savingSettings}
            label="Show floating pill"
            onChange={(showFloatingPill) =>
              actions.updateSettings({ showFloatingPill })
            }
          />
        </SettingRow>
        <SettingRow
          description="Display completion and failure notices."
          label="Notifications"
        >
          <Toggle
            checked={settings.notificationsEnabled}
            disabled={actions.savingSettings}
            label="Notifications"
            onChange={(notificationsEnabled) =>
              actions.updateSettings({ notificationsEnabled })
            }
          />
        </SettingRow>
        <SettingRow description="Play start and stop capture tones." label="Sounds">
          <Toggle
            checked={settings.soundsEnabled}
            disabled={actions.savingSettings}
            label="Sounds"
            onChange={(soundsEnabled) => actions.updateSettings({ soundsEnabled })}
          />
        </SettingRow>
      </SectionPanel>

      <SectionPanel
        icon={<SlidersHorizontal aria-hidden="true" size={16} />}
        title="Recording rules"
      >
        <SettingRow
          description="Choose which global capture modes are active."
          label="Recording mode"
        >
          <SegmentedControl
            disabled={actions.savingSettings}
            onChange={(recordingMode) => actions.updateSettings({ recordingMode })}
            options={recordingModeOptions}
            selected={settings.recordingMode}
          />
        </SettingRow>
        <SettingRow
          description="Trim leading and trailing quiet segments."
          label="Silence trim"
        >
          <Toggle
            checked={settings.silenceTrimEnabled}
            disabled={actions.savingSettings}
            label="Silence trim"
            onChange={(silenceTrimEnabled) =>
              actions.updateSettings({ silenceTrimEnabled })
            }
          />
        </SettingRow>
        <SettingRow
          description="Ignore accidental taps shorter than this."
          label="Minimum duration"
        >
          <input
            disabled={actions.savingSettings}
            min={1}
            onChange={(event) =>
              actions.updateSettings({
                minRecordingMs: Number(event.currentTarget.value),
              })
            }
            type="number"
            value={settings.minRecordingMs}
          />
        </SettingRow>
        <SettingRow
          description="Stop long recordings automatically."
          label="Maximum duration"
        >
          <input
            disabled={actions.savingSettings}
            min={settings.minRecordingMs}
            onChange={(event) =>
              actions.updateSettings({
                maxRecordingMs: Number(event.currentTarget.value),
              })
            }
            type="number"
            value={settings.maxRecordingMs}
          />
        </SettingRow>
      </SectionPanel>

      <SectionPanel
        icon={<MonitorCog aria-hidden="true" size={16} />}
        title="Data controls"
      >
        <div className="button-column">
          <button className="secondary-button" disabled type="button">
            <FolderOpen aria-hidden="true" size={15} />
            Open data folder pending
          </button>
          <button
            className="secondary-button"
            disabled={actions.clearingLastTranscript}
            onClick={() => void actions.clearLastTranscript()}
            type="button"
          >
            <Eraser aria-hidden="true" size={15} />
            {actions.clearingLastTranscript
              ? "Clearing..."
              : "Clear Last Transcript Buffer"}
          </button>
          <button className="ghost-button danger" disabled type="button">
            <Trash2 aria-hidden="true" size={15} />
            Reset settings pending
          </button>
        </div>
      </SectionPanel>
    </section>
  );
}

function HotkeysView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const hotkeys = hotkeyRows(settings);

  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Registered global hotkeys</h2>
          <CheckCircle2 aria-hidden="true" size={16} />
          <span className="pill pending">Registration pending</span>
        </div>
        <div className="hotkey-editor-list">
          {hotkeys.map((hotkey) => (
            <div className="hotkey-editor-row" key={hotkey.label}>
              <div>
                <strong>{hotkey.label}</strong>
                <span>Saved setting, backend registration pending</span>
              </div>
              <kbd>{formatHotkey(hotkey.value)}</kbd>
              <span className="pill pending">Pending</span>
              <button className="secondary-button" disabled type="button">
                <Keyboard aria-hidden="true" size={15} />
                Rebind pending
              </button>
            </div>
          ))}
        </div>
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Capture behavior</h2>
        </div>
        <SegmentedControl
          disabled={actions.savingSettings}
          onChange={(recordingMode) => actions.updateSettings({ recordingMode })}
          options={recordingModeOptions}
          selected={settings.recordingMode}
        />
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Conflict handling</h2>
        </div>
        <div className="pending-panel">
          <Info aria-hidden="true" size={16} />
          <span>Shortcut conflict detection will run with the hotkey service.</span>
        </div>
      </article>
    </section>
  );
}

function ModelsView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [progressByModel, setProgressByModel] = useState<
    Record<string, ModelDownloadProgress>
  >({});
  const [modelsLoading, setModelsLoading] = useState(true);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [busyModelId, setBusyModelId] = useState<string | null>(null);

  const loadModels = useCallback(async () => {
    setModelsLoading(true);
    setModelsError(null);

    try {
      setModels(await listModels());
    } catch (error) {
      setModelsError(commandErrorMessage(error));
    } finally {
      setModelsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadModels();
  }, [loadModels, settings.selectedModelId]);

  useEffect(() => {
    let disposed = false;
    let unlistenProgress: (() => void) | null = null;

    const setup = async () => {
      const unlisten = await listen<ModelDownloadProgress>(
        "model://download-progress",
        (event) => {
          setProgressByModel((current) => ({
            ...current,
            [event.payload.modelId]: event.payload,
          }));

          setModels((current) =>
            current.map((model) =>
              model.id === event.payload.modelId
                ? { ...model, status: event.payload.status }
                : model,
            ),
          );

          if (
            event.payload.status === "downloaded" ||
            event.payload.status === "selected" ||
            event.payload.status === "failed"
          ) {
            void loadModels();
          }
        },
      );
      unlistenProgress = unlisten;

      if (disposed) {
        unlisten();
      }
    };

    void setup();

    return () => {
      disposed = true;
      unlistenProgress?.();
    };
  }, [loadModels]);

  const runModelAction = useCallback(
    async (modelId: string, action: () => Promise<unknown>) => {
      setBusyModelId(modelId);
      setModelsError(null);

      try {
        await action();
        await loadModels();
        await actions.refresh();
      } catch (error) {
        setModelsError(commandErrorMessage(error));
      } finally {
        setBusyModelId(null);
      }
    },
    [actions, loadModels],
  );

  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Whisper models</h2>
          <button
            className="secondary-button"
            disabled={modelsLoading}
            onClick={() => void loadModels()}
            type="button"
          >
            <RefreshCw aria-hidden="true" size={15} />
            Refresh
          </button>
        </div>
        {modelsError ? (
          <InlineError message={modelsError} onRetry={loadModels} />
        ) : null}
        {modelsLoading ? (
          <div className="pending-panel">
            <RefreshCw aria-hidden="true" size={16} />
            <span>Loading model catalog...</span>
          </div>
        ) : null}
        {!modelsLoading && models.length === 0 ? (
          <EmptyState message="No Whisper models are available from the local catalog." />
        ) : null}
        <div className="model-table">
          <div className="model-table-header" aria-hidden="true">
            <span>Model</span>
            <span>Size</span>
            <span>Status</span>
            <span>Action</span>
          </div>
          {models.map((model) => {
            const progress = progressByModel[model.id];
            const status = progress?.status ?? model.status;
            const percent = progressPercent(model, progress);
            const isSelected = model.selected || model.id === settings.selectedModelId;
            const isDownloaded =
              status === "downloaded" ||
              status === "selected" ||
              status === "loaded";
            const isDownloading = status === "downloading";
            const isBusy = busyModelId === model.id;
            return (
              <div className="model-row" key={model.id}>
                <div>
                  <strong>{model.name}</strong>
                  <span>{model.filename}</span>
                  <div className="progress-track">
                    <div style={{ width: `${percent}%` }} />
                  </div>
                </div>
                <span>{model.diskSizeLabel}</span>
                <span className={modelStatusClass(status, isSelected)}>
                  {isSelected ? "Selected" : modelStatusLabel(status)}
                </span>
                <div className="row-actions">
                  {!isDownloaded && !isDownloading && status !== "failed" ? (
                    <button
                      className="secondary-button"
                      disabled={isBusy}
                      onClick={() =>
                        void runModelAction(model.id, () => downloadModel(model.id))
                      }
                      type="button"
                    >
                      <Download aria-hidden="true" size={15} />
                      {isBusy ? "Starting..." : "Download"}
                    </button>
                  ) : null}
                  {status === "failed" ? (
                    <button
                      className="secondary-button"
                      disabled={isBusy}
                      onClick={() =>
                        void runModelAction(model.id, () =>
                          retryModelDownload(model.id),
                        )
                      }
                      type="button"
                    >
                      <RefreshCw aria-hidden="true" size={15} />
                      Retry
                    </button>
                  ) : null}
                  {isDownloading ? (
                    <button
                      className="secondary-button"
                      disabled={isBusy}
                      onClick={() =>
                        void runModelAction(model.id, () =>
                          cancelModelDownload(model.id),
                        )
                      }
                      type="button"
                    >
                      <Square aria-hidden="true" size={15} />
                      Cancel
                    </button>
                  ) : null}
                  {isDownloaded ? (
                    <button
                      className="secondary-button"
                      disabled={isBusy || isSelected}
                      onClick={() =>
                        void runModelAction(model.id, () => selectModel(model.id))
                      }
                      type="button"
                    >
                      <CheckCircle2 aria-hidden="true" size={15} />
                      Select
                    </button>
                  ) : null}
                  {isDownloaded ? (
                    <IconButton
                      danger
                      disabled={isBusy || isDownloading}
                      label="Delete model"
                      onClick={() => {
                        if (
                          window.confirm(
                            `Delete ${model.name} from local model storage?`,
                          )
                        ) {
                          void runModelAction(model.id, () =>
                            deleteModel(model.id),
                          );
                        }
                      }}
                    >
                      <Trash2 aria-hidden="true" size={15} />
                    </IconButton>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Default model</h2>
        </div>
        <strong className="standout">
          {settings.selectedModelId ?? "No model selected"}
        </strong>
        <p className="muted">
          Downloaded models are stored under LocalDictate app data and resolved
          by the backend at runtime.
        </p>
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Storage</h2>
        </div>
        <code>LocalDictate app data / models</code>
        <div className="button-row">
          <button className="secondary-button" disabled type="button">
            <FolderOpen aria-hidden="true" size={15} />
            Open folder pending
          </button>
        </div>
      </article>
    </section>
  );
}

function AudioView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [microphones, setMicrophones] = useState<MicrophoneInfo[]>([]);
  const [microphonesLoading, setMicrophonesLoading] = useState(true);
  const [microphonesError, setMicrophonesError] = useState<string | null>(null);
  const [audioLevel, setAudioLevel] = useState(0);
  const [testingMic, setTestingMic] = useState(false);

  const loadMicrophones = useCallback(async () => {
    setMicrophonesLoading(true);
    setMicrophonesError(null);

    try {
      setMicrophones(await listMicrophones());
    } catch (error) {
      setMicrophonesError(commandErrorMessage(error));
    } finally {
      setMicrophonesLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadMicrophones();
  }, [loadMicrophones, settings.selectedMicId]);

  useEffect(() => {
    let disposed = false;
    let unlisteners: Array<() => void> = [];

    const setup = async () => {
      unlisteners = await Promise.all([
        listen<AudioLevelEvent>("audio://level", (event) => {
          setAudioLevel(Math.round(event.payload.level * 100));
        }),
        listen<RecordingErrorEvent>("audio://recording-error", (event) => {
          setMicrophonesError(event.payload.message);
        }),
      ]);

      if (disposed) {
        unlisteners.forEach((unlisten) => unlisten());
      }
    };

    void setup();

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  const selectedMicrophone = selectedMicrophoneLabel(
    microphones,
    settings.selectedMicId,
  );

  const handleRecordTestClip = useCallback(async () => {
    setTestingMic(true);
    setMicrophonesError(null);

    try {
      await recordTestClip(1600);
      await loadMicrophones();
      await actions.refresh();
    } catch (error) {
      setMicrophonesError(commandErrorMessage(error));
    } finally {
      setTestingMic(false);
    }
  }, [actions, loadMicrophones]);

  return (
    <section className="split-grid">
      <article className="buffer-card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Input</p>
            <h2>{selectedMicrophone}</h2>
          </div>
          <span
            className={
              microphonesError
                ? "pill error"
                : microphonesLoading
                  ? "pill pending"
                  : "pill ready"
            }
          >
            {microphonesError
              ? "Needs attention"
              : microphonesLoading
                ? "Scanning"
                : "Ready"}
          </span>
        </div>

        {microphonesError ? (
          <InlineError message={microphonesError} onRetry={loadMicrophones} />
        ) : null}
        <Waveform />
        <div className="meter">
          <div style={{ width: `${Math.max(4, audioLevel)}%` }} />
        </div>

        <div className="control-grid">
          <label>
            Microphone
            <select
              disabled={actions.savingSettings}
              onChange={(event) =>
                actions.updateSettings({
                  selectedMicId:
                    event.currentTarget.value === "default"
                      ? null
                      : event.currentTarget.value,
                })
              }
              value={settings.selectedMicId ?? "default"}
            >
              <option value="default">Default input device</option>
              {microphones.map((microphone) => (
                <option
                  disabled={!microphone.isAvailable}
                  key={microphone.id}
                  value={microphone.id}
                >
                  {microphone.name}
                  {microphone.isDefault ? " (default)" : ""}
                </option>
              ))}
            </select>
          </label>
          <label>
            Target format
            <input readOnly value="16 kHz mono PCM WAV" />
          </label>
        </div>

        <div className="button-row">
          <button
            className="primary-button"
            disabled={
              testingMic ||
              actions.recordingBusy ||
              microphonesLoading ||
              Boolean(microphonesError)
            }
            onClick={() => void handleRecordTestClip()}
            type="button"
          >
            <Mic aria-hidden="true" size={16} />
            {testingMic ? "Testing..." : "Record test"}
          </button>
          <button className="secondary-button" disabled type="button">
            <Play aria-hidden="true" size={15} />
            Playback pending
          </button>
        </div>
      </article>

      <div className="stack">
        <SectionPanel title="Audio processing">
          <SettingRow
            description="Remove quiet space around speech."
            label="Silence trim"
          >
            <Toggle
              checked={settings.silenceTrimEnabled}
              disabled={actions.savingSettings}
              label="Silence trim"
              onChange={(silenceTrimEnabled) =>
                actions.updateSettings({ silenceTrimEnabled })
              }
            />
          </SettingRow>
          <SettingRow
            description="Ignore captures below this length."
            label="Minimum duration"
          >
            <input
              disabled={actions.savingSettings}
              min={1}
              onChange={(event) =>
                actions.updateSettings({
                  minRecordingMs: Number(event.currentTarget.value),
                })
              }
              type="number"
              value={settings.minRecordingMs}
            />
          </SettingRow>
          <SettingRow
            description="Cap single dictation sessions."
            label="Maximum duration"
          >
            <input
              disabled={actions.savingSettings}
              min={settings.minRecordingMs}
              onChange={(event) =>
                actions.updateSettings({
                  maxRecordingMs: Number(event.currentTarget.value),
                })
              }
              type="number"
              value={settings.maxRecordingMs}
            />
          </SettingRow>
          <SettingRow
            description="Preferred file shape for transcription."
            label="Target format"
          >
            <select disabled value="wav">
              <option value="wav">16 kHz mono PCM WAV</option>
            </select>
          </SettingRow>
          <SettingRow description="Keep original clips for review." label="Save raw audio">
            <Toggle
              checked={settings.saveAudioClips}
              disabled={actions.savingSettings}
              label="Save raw audio"
              onChange={(saveAudioClips) =>
                actions.updateSettings({ saveAudioClips })
              }
            />
          </SettingRow>
        </SectionPanel>
        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Device health</h2>
            <span className={microphonesError ? "pill error" : "pill ready"}>
              {microphonesError ? "Unavailable" : "Available"}
            </span>
          </div>
          {microphonesLoading ? (
            <div className="pending-panel">
              <RefreshCw aria-hidden="true" size={16} />
              <span>Scanning Windows input devices...</span>
            </div>
          ) : null}
          {!microphonesLoading && microphones.length === 0 ? (
            <EmptyState message="No input devices were reported by Windows. Connect a microphone and refresh." />
          ) : null}
          {!microphonesLoading && microphones.length > 0 ? (
            <div className="device-list">
              {microphones.map((microphone) => (
                <div className="device-row" key={microphone.id}>
                  <div>
                    <strong>{microphone.name}</strong>
                    <span>{microphone.endpointId ?? microphone.id}</span>
                  </div>
                  <span
                    className={
                      microphone.isSelected
                        ? "pill selected"
                        : microphone.isDefault
                          ? "pill preserve"
                          : microphone.isAvailable
                            ? "pill ready"
                            : "pill error"
                    }
                  >
                    {microphone.isSelected
                      ? "Selected"
                      : microphone.isDefault
                        ? "Default"
                        : microphone.isAvailable
                          ? "Available"
                          : "Unavailable"}
                  </span>
                </div>
              ))}
            </div>
          ) : null}
        </article>
      </div>
    </section>
  );
}

function AboutView() {
  return (
    <section className="view-grid">
      <article className="buffer-card span-2">
        <div className="section-heading">
          <div>
            <p className="eyebrow">LocalDictate</p>
            <h2>Dictate locally without consuming your clipboard</h2>
          </div>
          <span className="pill preserve">Local-first</span>
        </div>
        <p className="transcript-text">
          LocalDictate is a Windows tray utility for private speech-to-text. It
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
          <strong>0.1.0</strong>
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
          <code>%APPDATA%/LocalDictate/</code>
        </SettingRow>
      </SectionPanel>
      <SectionPanel title="Resources">
        <div className="button-column">
          <button className="secondary-button" disabled type="button">
            <FolderOpen aria-hidden="true" size={15} />
            Open docs pending
          </button>
          <button className="secondary-button" disabled type="button">
            <Archive aria-hidden="true" size={15} />
            View licenses pending
          </button>
        </div>
      </SectionPanel>
    </section>
  );
}

function LastTranscriptCard({
  clearing,
  compact = false,
  copying,
  onClear,
  onCopy,
  onPaste,
  pasting,
  transcript,
}: {
  clearing: boolean;
  compact?: boolean;
  copying: boolean;
  onClear: () => Promise<void>;
  onCopy: () => Promise<void>;
  onPaste: () => Promise<void>;
  pasting: boolean;
  transcript: Transcript | null;
}) {
  const hasTranscript = Boolean(transcript);
  const outputBusy = clearing || copying || pasting;

  return (
    <article className={compact ? "panel-card" : "buffer-card"}>
      <div className="section-heading">
        <div>
          <p className="eyebrow">Last Transcript Buffer</p>
          <h2>{hasTranscript ? "Ready to insert later" : "No transcript stored"}</h2>
        </div>
        <span className="pill preserve">Clipboard Preserved</span>
      </div>

      {transcript ? (
        <>
          <p
            className={
              compact ? "transcript-text compact-text" : "transcript-text"
            }
          >
            {transcript.text}
          </p>

          <div className="metadata-row">
            <span>{formatCount(transcript.wordCount, "word")}</span>
            <span>{formatCount(transcript.characterCount, "char")}</span>
            <span>{formatDuration(transcript.durationMs)}</span>
            <span>{transcript.modelId ?? "No model recorded"}</span>
            <span>{formatDateTime(transcript.createdAt)}</span>
          </div>
        </>
      ) : (
        <EmptyState message="Complete a transcription to populate the buffer. Clipboard remains untouched." />
      )}

      <div className="button-row">
        <button
          className="primary-button"
          disabled={!hasTranscript || outputBusy}
          onClick={() => void onPaste()}
          type="button"
        >
          <ClipboardPaste aria-hidden="true" size={16} />
          {pasting ? "Inserting..." : "Insert"}
        </button>
        <button className="secondary-button" disabled type="button">
          <Pencil aria-hidden="true" size={15} />
          Edit pending
        </button>
        <button
          className="secondary-button"
          disabled={!hasTranscript || outputBusy}
          onClick={() => void onCopy()}
          type="button"
        >
          <Copy aria-hidden="true" size={15} />
          {copying ? "Copying..." : "Copy"}
        </button>
        <button
          className="ghost-button"
          disabled={!hasTranscript || outputBusy}
          onClick={() => void onClear()}
          type="button"
        >
          <Eraser aria-hidden="true" size={15} />
          {clearing ? "Clearing..." : "Clear"}
        </button>
      </div>
    </article>
  );
}

function RecentTranscriptsCard({
  recentTranscripts,
  setActiveView,
}: {
  recentTranscripts: Transcript[];
  setActiveView: (view: ViewName) => void;
}) {
  return (
    <article className="panel-card recent-card">
      <div className="section-heading compact">
        <h2>Recent Transcripts</h2>
        <button
          className="ghost-button"
          onClick={() => setActiveView("History")}
          type="button"
        >
          <Search aria-hidden="true" size={15} />
          Search
        </button>
      </div>
      {recentTranscripts.length === 0 ? (
        <EmptyState message="No saved transcript history yet." />
      ) : (
        <div className="transcript-list">
          {recentTranscripts.slice(0, 3).map((item) => (
            <TranscriptRow item={item} key={item.id} variant="compact" />
          ))}
        </div>
      )}
    </article>
  );
}

function StatsCard({ stats }: { stats: BasicStats }) {
  const statRows = [
    { label: "Words today", value: formatNumber(stats.wordsToday) },
    { label: "Dictations today", value: formatNumber(stats.dictationsToday) },
    { label: "Average WPM", value: formatOptionalNumber(stats.averageWpm) },
    {
      label: "Latency avg",
      value: formatOptionalDuration(stats.averageTranscriptionLatencyMs),
    },
    {
      label: "Recording avg",
      value: formatOptionalDuration(stats.averageRecordingDurationMs),
    },
    { label: "Most used model", value: stats.mostUsedModel ?? "None" },
    {
      label: "Total words",
      value: formatNumber(stats.totalWordsTranscribed),
    },
  ];

  return (
    <article className="panel-card">
      <div className="section-heading compact">
        <h2>Basic Stats</h2>
        <span className="muted">Local history</span>
      </div>
      <div className="stats-grid">
        {statRows.map((stat) => (
          <div className="stat-tile" key={stat.label}>
            <span>{stat.label}</span>
            <strong>{stat.value}</strong>
          </div>
        ))}
      </div>
    </article>
  );
}

function HotkeyList({
  compact = false,
  settings,
}: {
  compact?: boolean;
  settings: AppSettings;
}) {
  return (
    <div className={compact ? "hotkey-list compact-list" : "hotkey-list"}>
      {hotkeyRows(settings).map((hotkey) => (
        <div className="hotkey-row" key={hotkey.label}>
          <span>{hotkey.label}</span>
          <kbd>{formatHotkey(hotkey.value)}</kbd>
        </div>
      ))}
    </div>
  );
}

function StatusCard({
  action,
  Icon,
  label,
  onAction,
  status,
  value,
}: {
  action: string;
  Icon: LucideIcon;
  label: string;
  onAction: () => void;
  status: ReactNode;
  value: string;
}) {
  return (
    <article className="metric-card status-card">
      <div className="card-header">
        <span>
          <Icon aria-hidden="true" size={15} />
          {label}
        </span>
        {status}
      </div>
      <strong>{value}</strong>
      <button className="ghost-button" onClick={onAction} type="button">
        {action}
      </button>
    </article>
  );
}

function SectionPanel({
  children,
  icon,
  title,
}: {
  children: ReactNode;
  icon?: ReactNode;
  title: string;
}) {
  return (
    <article className="panel-card">
      <div className="section-heading compact">
        <h2>{title}</h2>
        {icon}
      </div>
      <div className="settings-list">{children}</div>
    </article>
  );
}

function SettingRow({
  children,
  description,
  label,
}: {
  children: ReactNode;
  description: string;
  label: string;
}) {
  return (
    <div className="settings-row">
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
      <div className="setting-control">{children}</div>
    </div>
  );
}

function TranscriptRow({
  busy = false,
  editText,
  item,
  onCancelEdit,
  onCopy,
  onDelete,
  onEdit,
  onEditTextChange,
  onPaste,
  onSaveEdit,
  variant,
}: {
  busy?: boolean;
  editText?: string;
  item: Transcript;
  onCancelEdit?: () => void;
  onCopy?: (id: string) => Promise<void>;
  onDelete?: (id: string) => Promise<void>;
  onEdit?: (transcript: Transcript) => void;
  onEditTextChange?: (text: string) => void;
  onPaste?: (id: string) => Promise<void>;
  onSaveEdit?: (id: string) => Promise<void>;
  variant: "compact" | "full";
}) {
  const isFull = variant === "full";
  const isEditing = editText !== undefined;

  return (
    <div className={isFull ? "history-row" : "transcript-row"}>
      <div>
        <strong>{transcriptTitle(item)}</strong>
        {isEditing ? (
          <textarea
            aria-label="Edit transcript text"
            className="history-edit"
            onChange={(event) => onEditTextChange?.(event.currentTarget.value)}
            value={editText}
          />
        ) : (
          <p>{item.text}</p>
        )}
        <span>{transcriptMeta(item)}</span>
      </div>
      <div className="row-actions">
        {isFull ? (
          <span className="pill preserve">
            {item.outputMode ? outputModeLabel(item.outputMode) : "Saved"}
          </span>
        ) : null}
        <button
          className={isFull ? "ghost-button" : "compact-action"}
          disabled={busy || !onPaste}
          onClick={() => void onPaste?.(item.id)}
          type="button"
        >
          <ClipboardPaste aria-hidden="true" size={15} />
          {busy ? "Working..." : "Insert"}
        </button>
        <button
          className={isFull ? "ghost-button" : "compact-action"}
          disabled={busy || !onCopy}
          onClick={() => void onCopy?.(item.id)}
          type="button"
        >
          <Copy aria-hidden="true" size={15} />
          Copy
        </button>
        {isFull ? (
          isEditing ? (
            <>
              <button
                className="secondary-button"
                disabled={busy || !editText.trim()}
                onClick={() => void onSaveEdit?.(item.id)}
                type="button"
              >
                Save
              </button>
              <button
                className="ghost-button"
                disabled={busy}
                onClick={onCancelEdit}
                type="button"
              >
                Cancel
              </button>
            </>
          ) : (
            <>
              <button
                className="ghost-button"
                disabled={busy || !onEdit}
                onClick={() => onEdit?.(item)}
                type="button"
              >
                <Pencil aria-hidden="true" size={15} />
                Edit
              </button>
              <button
                className="ghost-button danger"
                disabled={busy || !onDelete}
                onClick={() => void onDelete?.(item.id)}
                type="button"
              >
                <Trash2 aria-hidden="true" size={15} />
                Delete
              </button>
            </>
          )
        ) : null}
      </div>
    </div>
  );
}

function Toggle({
  checked,
  disabled = false,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      aria-label={label}
      aria-pressed={checked}
      className={checked ? "toggle is-on" : "toggle"}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      type="button"
    >
      <span />
    </button>
  );
}

function IconButton({
  children,
  danger = false,
  disabled = false,
  label,
  onClick,
}: {
  children: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  label: string;
  onClick?: () => void;
}) {
  return (
    <button
      aria-label={label}
      className={danger ? "icon-button danger" : "icon-button"}
      disabled={disabled}
      onClick={onClick}
      title={label}
      type="button"
    >
      {children}
    </button>
  );
}

function SegmentedControl<T extends string>({
  disabled = false,
  onChange,
  options,
  selected,
}: {
  disabled?: boolean;
  onChange: (selected: T) => void;
  options: { label: string; value: T }[];
  selected: T;
}) {
  return (
    <div className="segmented-control">
      {options.map((option) => (
        <button
          aria-pressed={option.value === selected}
          className={option.value === selected ? "active-segment" : ""}
          disabled={disabled}
          key={option.value}
          onClick={() => onChange(option.value)}
          type="button"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function StatePill({ appState }: { appState: AppStateSnapshot }) {
  const className = `pill ${stateTone(appState.status)}`;
  const label = appState.error?.message ?? appState.status;

  return (
    <span className={className} title={label}>
      {appState.status}
    </span>
  );
}

function FloatingPill({
  appState,
  outputMode,
  pasteHotkey,
}: {
  appState: AppStateSnapshot;
  outputMode: OutputMode;
  pasteHotkey: string;
}) {
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    if (appState.status === "Idle" || appState.status === "Paused") {
      setVisible(false);
      return;
    }

    setVisible(true);

    if (appState.status === "Ready") {
      const timer = window.setTimeout(() => setVisible(false), 5200);
      return () => window.clearTimeout(timer);
    }
  }, [appState.status, appState.updatedAt]);

  if (!visible) {
    return null;
  }

  const lines = floatingPillLines(appState, outputMode, pasteHotkey);

  return (
    <div className={`floating-pill ${stateTone(appState.status)}`} role="status">
      <span className="floating-pulse" aria-hidden="true" />
      <div>
        <strong>{lines[0]}</strong>
        {lines[1] ? <span>{lines[1]}</span> : null}
      </div>
    </div>
  );
}

function Toast({ notice }: { notice: ToastNotice }) {
  return (
    <div className={`toast-notice ${notice.tone}`} role="status">
      {notice.tone === "error" ? (
        <AlertCircle aria-hidden="true" size={16} />
      ) : (
        <CheckCircle2 aria-hidden="true" size={16} />
      )}
      <span>{notice.message}</span>
    </div>
  );
}

function LoadingPanel() {
  return (
    <article className="panel-card loading-panel">
      <RefreshCw aria-hidden="true" size={18} />
      <span>Loading dashboard data from LocalDictate commands...</span>
    </article>
  );
}

function ErrorPanel({
  message,
  onRetry,
}: {
  message: string | null;
  onRetry: () => Promise<void>;
}) {
  return (
    <article className="panel-card error-panel">
      <AlertCircle aria-hidden="true" size={18} />
      <div>
        <strong>Could not load backend data</strong>
        <p>{message ?? "The Tauri command layer did not return data."}</p>
      </div>
      <button className="secondary-button" onClick={() => void onRetry()} type="button">
        <RefreshCw aria-hidden="true" size={15} />
        Retry
      </button>
    </article>
  );
}

function InlineError({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => Promise<void>;
}) {
  return (
    <div className="inline-error">
      <AlertCircle aria-hidden="true" size={16} />
      <span>{message}</span>
      <button className="compact-action" onClick={() => void onRetry()} type="button">
        Refresh
      </button>
    </div>
  );
}

function EmptyState({ message }: { message: string }) {
  return (
    <div className="empty-state">
      <Info aria-hidden="true" size={16} />
      <span>{message}</span>
    </div>
  );
}

function Waveform() {
  return (
    <div className="recording-visual" aria-hidden="true">
      <span />
      <span />
      <span />
      <span />
      <span />
      <span />
      <span />
    </div>
  );
}

function clipboardStatus(settings: AppSettings) {
  if (
    settings.outputMode === "copy_clipboard" ||
    settings.outputMode === "copy_and_paste"
  ) {
    return "Copied to Clipboard";
  }

  if (
    settings.outputMode === "auto_paste" ||
    settings.pasteMethod === "clipboard_restore"
  ) {
    return "Clipboard Preserved";
  }

  return "Clipboard Untouched";
}

function outputModeLabel(outputMode: OutputMode) {
  return (
    outputModeOptions.find((option) => option.value === outputMode)?.label ??
    outputMode
  );
}

function routeToView(route: string): ViewName | null {
  const normalized = route.trim().toLowerCase();
  const routes: Record<string, ViewName> = {
    dashboard: "Dashboard",
    transcribe: "Transcribe",
    history: "History",
    settings: "Settings",
    hotkeys: "Hotkeys",
    models: "Models",
    audio: "Audio",
    about: "About",
  };

  return routes[normalized] ?? null;
}

function canStartRecording(data: DashboardData | null) {
  if (!data) {
    return false;
  }

  return (
    data.appState.status === "Idle" ||
    data.appState.status === "Ready" ||
    data.appState.status === "Error"
  );
}

function stateTone(status: AppStateSnapshot["status"]) {
  switch (status) {
    case "Recording":
      return "recording";
    case "Stopping":
    case "Transcribing":
    case "Pasting":
      return "pending";
    case "Ready":
      return "ready";
    case "Error":
      return "error";
    case "Paused":
      return "preserve";
    case "Idle":
    default:
      return "idle";
  }
}

function floatingPillLines(
  appState: AppStateSnapshot,
  outputMode: OutputMode,
  pasteHotkey: string,
) {
  if (appState.status === "Recording") {
    return ["Recording...", "Release hotkey or press Stop"];
  }

  if (appState.status === "Stopping") {
    return ["Saving audio...", "Preparing local transcription"];
  }

  if (appState.status === "Transcribing") {
    return ["Transcribing...", "Whisper is running locally"];
  }

  if (appState.status === "Pasting") {
    return ["Inserting transcript...", "Keeping clipboard behavior intact"];
  }

  if (appState.status === "Ready") {
    if (outputMode === "save_only") {
      return ["Saved to Last Transcript", "Clipboard preserved"];
    }

    return ["Transcript ready", `${formatHotkey(pasteHotkey)} to insert`];
  }

  if (appState.status === "Error") {
    return ["Needs attention", appState.error?.message ?? "Check LocalDictate"];
  }

  return ["Ready for dictation", ""];
}

function modelStatusLabel(status: ModelInfo["status"]) {
  const labels: Record<ModelInfo["status"], string> = {
    not_downloaded: "Not downloaded",
    downloading: "Downloading",
    downloaded: "Downloaded",
    selected: "Selected",
    loaded: "Loaded",
    failed: "Failed",
    update_available: "Update available",
  };

  return labels[status];
}

function modelStatusClass(status: ModelInfo["status"], selected: boolean) {
  if (selected || status === "selected" || status === "loaded") {
    return "pill selected";
  }

  if (status === "failed") {
    return "pill error";
  }

  if (status === "downloading" || status === "update_available") {
    return "pill pending";
  }

  if (status === "downloaded") {
    return "pill ready";
  }

  return "pill preserve";
}

function progressPercent(
  model: ModelInfo,
  progress: ModelDownloadProgress | undefined,
) {
  if (progress?.percent !== null && progress?.percent !== undefined) {
    return Math.max(0, Math.min(100, progress.percent));
  }

  if (
    model.status === "downloaded" ||
    model.status === "selected" ||
    model.status === "loaded"
  ) {
    return 100;
  }

  return 0;
}

function selectedMicrophoneLabel(
  microphones: MicrophoneInfo[],
  selectedMicId: string | null,
) {
  if (!selectedMicId) {
    return microphones.find((microphone) => microphone.isDefault)?.name ?? "Default input device";
  }

  return (
    microphones.find((microphone) => microphone.id === selectedMicId)?.name ??
    selectedMicId
  );
}

function recordingStageTitle(status: AppStateSnapshot["status"]) {
  switch (status) {
    case "Recording":
      return "Recording";
    case "Stopping":
      return "Stopping recording";
    case "Transcribing":
      return "Transcribing locally";
    case "Pasting":
      return "Inserting transcript";
    case "Ready":
      return "Transcript ready";
    case "Error":
      return "Needs attention";
    case "Paused":
      return "Paused";
    case "Idle":
    default:
      return "Ready for dictation";
  }
}

function hotkeyRows(settings: AppSettings) {
  return [
    { label: "Hold-to-Talk", value: settings.hotkeys.holdToTalk },
    { label: "Toggle Dictation", value: settings.hotkeys.toggleDictation },
    { label: "Paste Last", value: settings.hotkeys.pasteLastTranscript },
    { label: "Open Dashboard", value: settings.hotkeys.openDashboard },
  ];
}

function retentionToValue(retention: HistoryRetentionDays) {
  return retention === null ? "forever" : String(retention);
}

function retentionFromValue(value: string): HistoryRetentionDays {
  if (value === "forever") {
    return null;
  }

  const numeric = Number(value);
  return numeric === 7 || numeric === 30 || numeric === 90 || numeric === 365
    ? numeric
    : 30;
}

function formatHotkey(value: string) {
  return value.split("+").join(" + ");
}

function formatDateTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "Unknown time";
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

function formatDuration(milliseconds: number | null) {
  if (milliseconds === null) {
    return "No audio duration";
  }

  if (milliseconds < 1000) {
    return `${milliseconds} ms audio`;
  }

  return `${(milliseconds / 1000).toFixed(1)}s audio`;
}

function formatOptionalDuration(milliseconds: number | null) {
  if (milliseconds === null) {
    return "None";
  }

  if (milliseconds < 1000) {
    return `${Math.round(milliseconds)} ms`;
  }

  return `${(milliseconds / 1000).toFixed(1)}s`;
}

function formatOptionalNumber(value: number | null) {
  return value === null ? "None" : Math.round(value).toLocaleString();
}

function formatNumber(value: number) {
  return value.toLocaleString();
}

function formatCount(count: number, unit: string) {
  return `${formatNumber(count)} ${count === 1 ? unit : `${unit}s`}`;
}

function transcriptTitle(transcript: Transcript) {
  return `Transcript from ${formatDateTime(transcript.createdAt)}`;
}

function transcriptMeta(transcript: Transcript) {
  return [
    formatCount(transcript.wordCount, "word"),
    transcript.modelId ?? "No model",
    formatDuration(transcript.durationMs),
  ].join(" | ");
}

export default App;
