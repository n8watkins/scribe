import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
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
  commandErrorMessage,
  getDashboardData,
  updateSettings,
  type AppSettings,
  type AppStateSnapshot,
  type BasicStats,
  type DashboardData,
  type HistoryRetentionDays,
  type OutputMode,
  type PasteMethod,
  type RecordingMode,
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
  clearLastTranscript: () => Promise<void>;
  clearingLastTranscript: boolean;
  refresh: () => Promise<void>;
  saveError: string | null;
  savingSettings: boolean;
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

const modelRows = [
  {
    name: "small.en quantized",
    id: "small.en-q5_1",
    size: "181 MB",
    status: "Downloaded",
    progress: 100,
  },
  {
    name: "base.en",
    id: "base.en",
    size: "142 MB",
    status: "Downloaded",
    progress: 100,
  },
  {
    name: "medium.en quantized",
    id: "medium.en-q5_0",
    size: "514 MB",
    status: "Pending service",
    progress: 0,
  },
  {
    name: "large-v3-turbo quantized",
    id: "large-v3-turbo-q5_0",
    size: "1.6 GB",
    status: "Pending service",
    progress: 0,
  },
];

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
  const heading = viewTitles[activeView];

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

  const actions: ViewActions = {
    clearLastTranscript: handleClearLastTranscript,
    clearingLastTranscript,
    refresh,
    saveError,
    savingSettings,
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
              disabled
              title="Recording commands are pending the audio service."
              type="button"
            >
              <Mic aria-hidden="true" size={16} />
              Start pending
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
          onClear={actions.clearLastTranscript}
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
              mode. Recording commands are pending the audio service.
            </p>
          </div>
        </div>

        <div className="button-row">
          <button className="primary-button" disabled type="button">
            <Mic aria-hidden="true" size={16} />
            Start pending
          </button>
          <button className="secondary-button" disabled type="button">
            <Square aria-hidden="true" size={15} />
            Stop pending
          </button>
          <button className="ghost-button" disabled type="button">
            <Eraser aria-hidden="true" size={15} />
            Cancel pending
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
          onClear={actions.clearLastTranscript}
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
  const { recentTranscripts, settings } = data;
  const filteredTranscripts = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) {
      return recentTranscripts;
    }

    return recentTranscripts.filter((transcript) =>
      transcript.text.toLowerCase().includes(normalizedQuery),
    );
  }, [query, recentTranscripts]);

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
          <button className="secondary-button" disabled type="button">
            <Trash2 aria-hidden="true" size={15} />
            Clear all pending
          </button>
        </div>
      </article>

      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Transcript archive</h2>
          <Archive aria-hidden="true" size={16} />
          <span className="muted">
            {filteredTranscripts.length} local records
          </span>
        </div>
        {settings.historyEnabled ? null : (
          <EmptyState message="History is disabled. The Last Transcript Buffer can still hold the most recent result." />
        )}
        {settings.historyEnabled && filteredTranscripts.length === 0 ? (
          <EmptyState message="No local transcript records match this view yet." />
        ) : null}
        {settings.historyEnabled && filteredTranscripts.length > 0 ? (
          <div className="transcript-list">
            {filteredTranscripts.map((item) => (
              <TranscriptRow item={item} key={item.id} variant="full" />
            ))}
          </div>
        ) : null}
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
  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Whisper models</h2>
          <button className="secondary-button" disabled type="button">
            <FolderOpen aria-hidden="true" size={15} />
            Open folder pending
          </button>
        </div>
        <div className="model-table">
          <div className="model-table-header" aria-hidden="true">
            <span>Model</span>
            <span>Size</span>
            <span>Status</span>
            <span>Action</span>
          </div>
          {modelRows.map((model) => {
            const isSelected = model.id === settings.selectedModelId;
            return (
              <div className="model-row" key={model.id}>
                <div>
                  <strong>{model.name}</strong>
                  <span>{model.id}</span>
                  <div className="progress-track">
                    <div style={{ width: `${model.progress}%` }} />
                  </div>
                </div>
                <span>{model.size}</span>
                <span className={isSelected ? "pill selected" : "pill preserve"}>
                  {isSelected ? "Selected" : model.status}
                </span>
                <div className="row-actions">
                  {model.progress === 0 ? (
                    <button className="secondary-button" disabled type="button">
                      <Download aria-hidden="true" size={15} />
                      Download pending
                    </button>
                  ) : null}
                  {model.progress === 100 ? (
                    <button
                      className="secondary-button"
                      disabled={actions.savingSettings || isSelected}
                      onClick={() =>
                        actions.updateSettings({ selectedModelId: model.id })
                      }
                      type="button"
                    >
                      <CheckCircle2 aria-hidden="true" size={15} />
                      Select
                    </button>
                  ) : null}
                  {model.progress === 100 ? (
                    <IconButton danger disabled label="Delete pending">
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
          Model discovery and download state will come from the model manager.
        </p>
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Storage</h2>
        </div>
        <code>%APPDATA%/LocalDictate/models/</code>
        <div className="button-row">
          <button className="secondary-button" disabled type="button">
            <FolderOpen aria-hidden="true" size={15} />
            Open pending
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
  return (
    <section className="split-grid">
      <article className="buffer-card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Input</p>
            <h2>{settings.selectedMicId ?? "Default communications device"}</h2>
          </div>
          <span className="pill pending">Device list pending</span>
        </div>

        <Waveform />
        <div className="meter">
          <div />
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
              <option value="default">Default communications device</option>
              <option value="usb">USB microphone</option>
              <option value="array">Microphone array</option>
            </select>
          </label>
          <label>
            Target format
            <input readOnly value="16 kHz mono PCM WAV" />
          </label>
        </div>

        <div className="button-row">
          <button className="primary-button" disabled type="button">
            <Mic aria-hidden="true" size={16} />
            Test pending
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
            <span className="pill pending">Pending audio service</span>
          </div>
          <p className="muted">
            Permission, unavailable device, and recording failure states will
            surface here from the Rust audio service.
          </p>
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
  onClear,
  transcript,
}: {
  clearing: boolean;
  compact?: boolean;
  onClear: () => Promise<void>;
  transcript: Transcript | null;
}) {
  const hasTranscript = Boolean(transcript);

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
        <button className="primary-button" disabled type="button">
          <ClipboardPaste aria-hidden="true" size={16} />
          Insert pending
        </button>
        <button className="secondary-button" disabled type="button">
          <Pencil aria-hidden="true" size={15} />
          Edit pending
        </button>
        <button className="secondary-button" disabled type="button">
          <Copy aria-hidden="true" size={15} />
          Copy pending
        </button>
        <button
          className="ghost-button"
          disabled={!hasTranscript || clearing}
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
  item,
  variant,
}: {
  item: Transcript;
  variant: "compact" | "full";
}) {
  const isFull = variant === "full";

  return (
    <div className={isFull ? "history-row" : "transcript-row"}>
      <div>
        <strong>{transcriptTitle(item)}</strong>
        <p>{item.text}</p>
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
          disabled
          type="button"
        >
          <ClipboardPaste aria-hidden="true" size={15} />
          Insert pending
        </button>
        <button
          className={isFull ? "ghost-button" : "compact-action"}
          disabled
          type="button"
        >
          <Copy aria-hidden="true" size={15} />
          Copy pending
        </button>
        {isFull ? (
          <>
            <button className="ghost-button" disabled type="button">
              <Pencil aria-hidden="true" size={15} />
              Edit pending
            </button>
            <button className="ghost-button danger" disabled type="button">
              <Trash2 aria-hidden="true" size={15} />
              Delete pending
            </button>
          </>
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
}: {
  children: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      aria-label={label}
      className={danger ? "icon-button danger" : "icon-button"}
      disabled={disabled}
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
  const className = appState.status === "Error" ? "pill error" : "pill ready";
  const label = appState.error?.message ?? appState.status;

  return (
    <span className={className} title={label}>
      {appState.status}
    </span>
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
