import { useState } from "react";
import {
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
  Search,
  Settings as SettingsIcon,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  Trash2,
  type LucideIcon,
} from "lucide-react";
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

const hotkeys = [
  { label: "Hold-to-Talk", value: "Ctrl + Win + Space", status: "Ready" },
  { label: "Toggle Dictation", value: "Ctrl + Win + D", status: "Ready" },
  { label: "Paste Last", value: "Ctrl + Alt + V", status: "Ready" },
  { label: "Open Dashboard", value: "Ctrl + Win + H", status: "Ready" },
];

const recentTranscripts = [
  {
    title: "Project status note",
    text: "The core product promise is clipboard-safe local dictation with a reusable last transcript buffer.",
    meta: "142 words | small.en-q5_1 | 10:42 AM",
    output: "Save Only",
  },
  {
    title: "Email draft",
    text: "Can you review the implementation plan and confirm which Windows paste path we should validate first?",
    meta: "31 words | small.en-q5_1 | 9:18 AM",
    output: "Auto Paste",
  },
  {
    title: "Meeting capture",
    text: "Prioritize hotkey reliability, audio normalization, and the first recording to transcription slice.",
    meta: "24 words | base.en | Yesterday",
    output: "Save Only",
  },
];

const stats = [
  { label: "Words today", value: "1,284" },
  { label: "Dictations today", value: "18" },
  { label: "Average WPM", value: "132" },
  { label: "Latency avg", value: "1.8s" },
];

const models = [
  {
    name: "small.en quantized",
    id: "small.en-q5_1",
    size: "181 MB",
    status: "Selected",
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
    name: "large-v3-turbo quantized",
    id: "large-v3-turbo-q5_0",
    size: "1.6 GB",
    status: "Not Downloaded",
    progress: 0,
  },
];

function App() {
  const [activeView, setActiveView] = useState<ViewName>("Dashboard");
  const heading = viewTitles[activeView];

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
              onClick={() => setActiveView("Transcribe")}
              type="button"
            >
              <Mic aria-hidden="true" size={16} />
              Start dictation
            </button>
          </div>
        </header>

        {renderView(activeView, setActiveView)}
      </main>
    </div>
  );
}

function renderView(
  activeView: ViewName,
  setActiveView: (view: ViewName) => void,
) {
  switch (activeView) {
    case "Transcribe":
      return <TranscribeView />;
    case "History":
      return <HistoryView />;
    case "Settings":
      return <SettingsView />;
    case "Hotkeys":
      return <HotkeysView />;
    case "Models":
      return <ModelsView />;
    case "Audio":
      return <AudioView />;
    case "About":
      return <AboutView />;
    case "Dashboard":
    default:
      return <DashboardView setActiveView={setActiveView} />;
  }
}

function DashboardView({
  setActiveView,
}: {
  setActiveView: (view: ViewName) => void;
}) {
  return (
    <>
      <section className="status-grid" aria-label="Current setup">
        <article className="metric-card recording-card">
          <div className="card-header">
            <span>
              <Gauge aria-hidden="true" size={15} />
              Current Status
            </span>
            <span className="pill ready">Ready</span>
          </div>
          <Waveform />
          <p className="muted">Waiting for hold-to-talk or toggle dictation.</p>
        </article>

        <article className="metric-card">
          <div className="card-header">
            <span>
              <Mic aria-hidden="true" size={15} />
              Active Microphone
            </span>
            <span className="status-dot success" />
          </div>
          <strong>Default communications device</strong>
          <p className="muted">Input level meter and selector wire in during Phase 2.</p>
        </article>

        <article className="metric-card">
          <div className="card-header">
            <span>
              <Database aria-hidden="true" size={15} />
              Active Whisper Model
            </span>
            <span className="pill selected">Selected</span>
          </div>
          <strong>small.en quantized</strong>
          <p className="muted">Model manager will download to app data storage.</p>
        </article>

        <article className="metric-card">
          <div className="card-header">
            <span>
              <Clipboard aria-hidden="true" size={15} />
              Output Mode
            </span>
            <span className="pill preserve">Clipboard Untouched</span>
          </div>
          <strong>Save Only</strong>
          <p className="muted">Paste last transcript stays separate from the clipboard.</p>
        </article>
      </section>

      <section className="main-grid">
        <LastTranscriptCard />

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
          <HotkeyList compact />
        </article>

        <RecentTranscriptsCard setActiveView={setActiveView} />

        <StatsCard />
      </section>
    </>
  );
}

function TranscribeView() {
  return (
    <section className="split-grid">
      <article className="buffer-card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Recording</p>
            <h2>Push-to-talk capture</h2>
          </div>
          <span className="pill ready">Idle</span>
        </div>

        <div className="recording-stage">
          <Waveform />
          <div>
            <strong>Ready for dictation</strong>
            <p className="muted">
              Hold Ctrl + Win + Space or use toggle mode. Backend recording will
              stream input level here once the audio service is wired.
            </p>
          </div>
        </div>

        <div className="button-row">
          <button className="primary-button" type="button">
            <Mic aria-hidden="true" size={16} />
            Start recording
          </button>
          <button className="secondary-button" type="button">
            <Square aria-hidden="true" size={15} />
            Stop and transcribe
          </button>
          <button className="ghost-button" type="button">
            <Eraser aria-hidden="true" size={15} />
            Cancel
          </button>
        </div>
      </article>

      <div className="stack">
        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Output behavior</h2>
            <span className="pill preserve">Clipboard Untouched</span>
          </div>
          <SegmentedControl
            options={["Save Only", "Auto Paste", "Copy", "Copy + Paste"]}
            selected="Save Only"
          />
        </article>

        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Paste method</h2>
          </div>
          <div className="choice-list">
            <label className="choice-row selected-choice">
              <input defaultChecked name="paste-method" type="radio" />
              <span>
                <strong>Direct Insert</strong>
                <small>Preserve the clipboard by default.</small>
              </span>
            </label>
            <label className="choice-row">
              <input name="paste-method" type="radio" />
              <span>
                <strong>Compatibility Paste</strong>
                <small>Temporarily use clipboard, then restore it.</small>
              </span>
            </label>
          </div>
        </article>

        <LastTranscriptCard compact />
      </div>
    </section>
  );
}

function HistoryView() {
  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="toolbar-row">
          <div className="search-field">
            <Search aria-hidden="true" size={16} />
            <input aria-label="Search transcripts" placeholder="Search transcripts" />
          </div>
          <select aria-label="Retention">
            <option>30 day retention</option>
            <option>7 day retention</option>
            <option>90 day retention</option>
            <option>Forever</option>
          </select>
          <button className="secondary-button" type="button">
            <Trash2 aria-hidden="true" size={15} />
            Clear all
          </button>
        </div>
      </article>

      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Transcript archive</h2>
          <Archive aria-hidden="true" size={16} />
          <span className="muted">3 local records</span>
        </div>
        <div className="transcript-list">
          {recentTranscripts.map((item) => (
            <div className="history-row" key={item.title}>
              <div>
                <strong>{item.title}</strong>
                <p>{item.text}</p>
                <span>{item.meta}</span>
              </div>
              <div className="row-actions">
                <span className="pill preserve">{item.output}</span>
                <button className="ghost-button" type="button">
                  <ClipboardPaste aria-hidden="true" size={15} />
                  Insert
                </button>
                <button className="ghost-button" type="button">
                  <Copy aria-hidden="true" size={15} />
                  Copy
                </button>
              </div>
            </div>
          ))}
        </div>
      </article>
    </section>
  );
}

function SettingsView() {
  return (
    <section className="view-grid">
      <SettingsPanel
        title="Privacy defaults"
        rows={[
          ["Cloud transcription", "Not present"],
          ["Save transcript history", "Enabled"],
          ["Save raw audio", "Disabled"],
          ["Telemetry", "Disabled"],
        ]}
      />
      <SettingsPanel
        title="Feedback"
        rows={[
          ["Floating recording pill", "Enabled"],
          ["Native notifications", "Enabled"],
          ["Start/stop sounds", "Disabled"],
          ["Minimize to tray", "Enabled"],
        ]}
      />
      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Recording rules</h2>
          <SlidersHorizontal aria-hidden="true" size={16} />
        </div>
        <div className="control-grid">
          <label>
            Recording mode
            <select defaultValue="both">
              <option value="hold">Hold-to-talk</option>
              <option value="toggle">Toggle start/stop</option>
              <option value="both">Both enabled</option>
            </select>
          </label>
          <label>
            Minimum duration
            <input defaultValue="300 ms" />
          </label>
          <label>
            Maximum duration
            <input defaultValue="3 minutes" />
          </label>
          <label>
            Language
            <select defaultValue="en">
              <option value="auto">Auto detect</option>
              <option value="en">English</option>
            </select>
          </label>
        </div>
      </article>
      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Data controls</h2>
          <MonitorCog aria-hidden="true" size={16} />
        </div>
        <div className="button-column">
          <button className="secondary-button" type="button">
            <FolderOpen aria-hidden="true" size={15} />
            Open local data folder
          </button>
          <button className="secondary-button" type="button">
            <Eraser aria-hidden="true" size={15} />
            Clear Last Transcript Buffer
          </button>
          <button className="ghost-button danger" type="button">
            <Trash2 aria-hidden="true" size={15} />
            Reset all settings
          </button>
        </div>
      </article>
    </section>
  );
}

function HotkeysView() {
  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Registered global hotkeys</h2>
          <CheckCircle2 aria-hidden="true" size={16} />
          <span className="pill ready">All valid</span>
        </div>
        <div className="hotkey-editor-list">
          {hotkeys.map((hotkey) => (
            <div className="hotkey-editor-row" key={hotkey.label}>
              <div>
                <strong>{hotkey.label}</strong>
                <span>{hotkey.status}</span>
              </div>
              <kbd>{hotkey.value}</kbd>
              <button className="secondary-button" type="button">
                <Keyboard aria-hidden="true" size={15} />
                Rebind
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
          options={["Hold-to-talk", "Toggle", "Both enabled"]}
          selected="Both enabled"
        />
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Conflict handling</h2>
        </div>
        <p className="muted">
          Registration failures will preserve the previous shortcut and show a
          specific conflict message once the backend hotkey service is wired.
        </p>
      </article>
    </section>
  );
}

function ModelsView() {
  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Whisper models</h2>
          <button className="secondary-button" type="button">
            <FolderOpen aria-hidden="true" size={15} />
            Open model folder
          </button>
        </div>
        <div className="model-list">
          {models.map((model) => (
            <div className="model-row" key={model.id}>
              <div>
                <strong>{model.name}</strong>
                <span>
                  {model.id} | {model.size}
                </span>
                <div className="progress-track">
                  <div style={{ width: `${model.progress}%` }} />
                </div>
              </div>
              <span
                className={
                  model.status === "Selected" ? "pill selected" : "pill preserve"
                }
              >
                {model.status}
              </span>
              <button className="secondary-button" type="button">
                {model.progress === 100 ? (
                  <CheckCircle2 aria-hidden="true" size={15} />
                ) : (
                  <Download aria-hidden="true" size={15} />
                )}
                {model.progress === 100 ? "Select" : "Download"}
              </button>
            </div>
          ))}
        </div>
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Default model</h2>
        </div>
        <strong className="standout">small.en quantized</strong>
        <p className="muted">
          Balanced quality and speed for daily English dictation.
        </p>
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Storage</h2>
        </div>
        <code>%APPDATA%/LocalDictate/models/</code>
        <p className="muted">Downloads stay local and can be deleted anytime.</p>
      </article>
    </section>
  );
}

function AudioView() {
  return (
    <section className="split-grid">
      <article className="buffer-card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Input</p>
            <h2>Default communications device</h2>
          </div>
          <span className="status-dot success" />
        </div>

        <Waveform />
        <div className="meter">
          <div />
        </div>

        <div className="control-grid">
          <label>
            Microphone
            <select defaultValue="default">
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
          <button className="primary-button" type="button">
            <Mic aria-hidden="true" size={16} />
            Test recording
          </button>
          <button className="secondary-button" type="button">
            <Play aria-hidden="true" size={15} />
            Play test
          </button>
        </div>
      </article>

      <div className="stack">
        <SettingsPanel
          title="Audio processing"
          rows={[
            ["Silence trim", "Enabled"],
            ["Minimum duration", "300 ms"],
            ["Maximum duration", "3 minutes"],
            ["Raw audio history", "Disabled"],
          ]}
        />
        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Device health</h2>
            <span className="pill ready">Available</span>
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

      <SettingsPanel
        title="V1 boundaries"
        rows={[
          ["Cloud transcription", "Non-goal"],
          ["Wake word", "Non-goal"],
          ["Realtime streaming", "Non-goal"],
          ["Command mode", "Non-goal"],
        ]}
      />
      <SettingsPanel
        title="Build phases"
        rows={[
          ["Current", "Skeleton app"],
          ["Next", "State, tray, settings"],
          ["Then", "Hotkeys and recording"],
          ["After", "Whisper and paste"],
        ]}
      />
    </section>
  );
}

function LastTranscriptCard({ compact = false }: { compact?: boolean }) {
  return (
    <article className={compact ? "panel-card" : "buffer-card"}>
      <div className="section-heading">
        <div>
          <p className="eyebrow">Last Transcript Buffer</p>
          <h2>Ready to insert later</h2>
        </div>
        <span className="pill preserve">Clipboard Preserved</span>
      </div>

      <p className={compact ? "transcript-text compact-text" : "transcript-text"}>
        This is the most recent dictated text. It is stored inside LocalDictate,
        separate from the system clipboard, and can be inserted with the
        paste-last hotkey.
      </p>

      <div className="metadata-row">
        <span>42 words</span>
        <span>231 chars</span>
        <span>8.4s audio</span>
        <span>small.en-q5_1</span>
      </div>

      <div className="button-row">
        <button className="primary-button" type="button">
          <ClipboardPaste aria-hidden="true" size={16} />
          Insert
        </button>
        <button className="secondary-button" type="button">
          <Pencil aria-hidden="true" size={15} />
          Edit
        </button>
        <button className="secondary-button" type="button">
          <Copy aria-hidden="true" size={15} />
          Copy
        </button>
        <button className="ghost-button" type="button">
          <Eraser aria-hidden="true" size={15} />
          Clear
        </button>
      </div>
    </article>
  );
}

function RecentTranscriptsCard({
  setActiveView,
}: {
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
      <div className="transcript-list">
        {recentTranscripts.map((item) => (
          <div className="transcript-row" key={item.title}>
            <div>
              <strong>{item.title}</strong>
              <p>{item.text}</p>
            </div>
            <span>{item.meta}</span>
          </div>
        ))}
      </div>
    </article>
  );
}

function StatsCard() {
  return (
    <article className="panel-card">
      <div className="section-heading compact">
        <h2>Basic Stats</h2>
        <span className="muted">Today</span>
      </div>
      <div className="stats-grid">
        {stats.map((stat) => (
          <div className="stat-tile" key={stat.label}>
            <span>{stat.label}</span>
            <strong>{stat.value}</strong>
          </div>
        ))}
      </div>
    </article>
  );
}

function HotkeyList({ compact = false }: { compact?: boolean }) {
  return (
    <div className={compact ? "hotkey-list compact-list" : "hotkey-list"}>
      {hotkeys.map((hotkey) => (
        <div className="hotkey-row" key={hotkey.label}>
          <span>{hotkey.label}</span>
          <kbd>{hotkey.value}</kbd>
        </div>
      ))}
    </div>
  );
}

function SettingsPanel({
  title,
  rows,
}: {
  title: string;
  rows: [string, string][];
}) {
  return (
    <article className="panel-card">
      <div className="section-heading compact">
        <h2>{title}</h2>
      </div>
      <div className="settings-list">
        {rows.map(([label, value]) => (
          <div className="settings-row" key={label}>
            <span>{label}</span>
            <strong>{value}</strong>
          </div>
        ))}
      </div>
    </article>
  );
}

function SegmentedControl({
  options,
  selected,
}: {
  options: string[];
  selected: string;
}) {
  return (
    <div className="segmented-control">
      {options.map((option) => (
        <button
          className={option === selected ? "active-segment" : ""}
          key={option}
          type="button"
        >
          {option}
        </button>
      ))}
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

export default App;
