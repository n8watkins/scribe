import { useState } from "react";
import {
  BookOpen,
  ClipboardPaste,
  Filter,
  Languages,
  NotebookPen,
  PanelTop,
  Palette,
  Repeat2,
  SlidersHorizontal,
  Sparkles,
  Wand2,
} from "lucide-react";
import type { AppSettings, DashboardData } from "../backend";
import type { ToastNotice, ViewActions } from "../types";
import { formatHotkey } from "../lib/format";
import { SectionPanel } from "../components/layout";
import { OutputSettingRows } from "../components/OutputSettings";
import { TransformSelectionPanel } from "../components/TransformSelectionPanel";
import {
  AppBehaviorSettings,
  ContextHintSettings,
  FillerSettings,
  LanguageSettings,
  LocalAiSettings,
  PillColorSettings,
  PillSettings,
  ReplacementsSettings,
} from "./Settings";
import { GpuSettings, MicrophoneView } from "./Audio";
import { DataPrivacyView, WindowSizeSettings } from "./DataPrivacy";
import { HistoryView } from "./History";
import { ModelsView } from "./Models";
import { SyncView } from "./Sync";
import { ThemesView } from "./Themes";
import { FileTranscribeCard } from "./Transcribe";

// One page per sidebar entry. Pages only arrange panels; the setting rows
// themselves live with the views they came from.

type PageProps = { actions: ViewActions; settings: AppSettings };

const HISTORY_TABS = [
  { id: "dictations", title: "Dictations", Icon: Repeat2 },
  { id: "notes", title: "Notes", Icon: NotebookPen },
] as const;

export function HistoryPage({
  actions,
  data,
}: {
  actions: ViewActions;
  data: DashboardData;
}) {
  const [tab, setTab] = useState<(typeof HISTORY_TABS)[number]["id"]>(
    "dictations",
  );
  return (
    <div className="stack">
      <div
        aria-label="History"
        className="settings-tabs page-tabs"
        data-setting="History"
        role="tablist"
      >
        {HISTORY_TABS.map(({ id, title, Icon }) => (
          <button
            aria-controls="history-tabpanel"
            aria-selected={tab === id}
            className={`settings-tab${tab === id ? " is-active" : ""}`}
            id={`history-tab-${id}`}
            key={id}
            onClick={() => setTab(id)}
            role="tab"
            type="button"
          >
            <Icon aria-hidden="true" size={15} />
            <span>{title}</span>
          </button>
        ))}
      </div>
      <div
        aria-labelledby={`history-tab-${tab}`}
        id="history-tabpanel"
        role="tabpanel"
      >
        <HistoryView
          actions={actions}
          data={data}
          key={tab}
          notesOnly={tab === "notes"}
        />
      </div>
    </div>
  );
}

export function TranscribeFilePage() {
  return <FileTranscribeCard />;
}

export function DictationPage({ actions, settings }: PageProps) {
  return (
    <section className="stack">
      <SectionPanel
        icon={<ClipboardPaste aria-hidden="true" size={16} />}
        title="After you dictate"
      >
        <OutputSettingRows
          disabled={actions.savingSettings}
          onChange={(patch) => actions.updateSettings(patch)}
          settings={settings}
        />
      </SectionPanel>
      <SectionPanel
        icon={<PanelTop aria-hidden="true" size={16} />}
        title="Floating pill"
      >
        <PillSettings actions={actions} settings={settings} />
      </SectionPanel>
    </section>
  );
}

export function WordsPage({ actions, settings }: PageProps) {
  return (
    <section className="split-grid">
      <div className="stack">
        <SectionPanel
          icon={<BookOpen aria-hidden="true" size={16} />}
          title="Context hint"
        >
          <ContextHintSettings actions={actions} settings={settings} />
        </SectionPanel>
        <SectionPanel
          icon={<Filter aria-hidden="true" size={16} />}
          title="Filler words"
        >
          <FillerSettings actions={actions} settings={settings} />
        </SectionPanel>
      </div>
      <SectionPanel
        icon={<Repeat2 aria-hidden="true" size={16} />}
        title="Replacements"
      >
        <ReplacementsSettings actions={actions} settings={settings} />
      </SectionPanel>
    </section>
  );
}

export function MicrophonePage({ actions, settings }: PageProps) {
  return <MicrophoneView actions={actions} settings={settings} />;
}

export function ModelsAndLanguagePage({ actions, settings }: PageProps) {
  return (
    <ModelsView actions={actions} settings={settings}>
      <SectionPanel
        icon={<Languages aria-hidden="true" size={16} />}
        title="Language"
      >
        <LanguageSettings actions={actions} settings={settings} />
      </SectionPanel>
      <GpuSettings actions={actions} settings={settings} />
    </ModelsView>
  );
}

export function LocalAiPage({
  actions,
  settings,
  showNotice,
}: PageProps & {
  showNotice: (message: string, tone?: ToastNotice["tone"]) => void;
}) {
  const on = settings.notesAnalysisEnabled;
  const state = (
    <span className={on ? "pill ready" : "pill idle"}>{on ? "On" : "Off"}</span>
  );
  return (
    <section className="split-grid">
      <SectionPanel
        icon={<Sparkles aria-hidden="true" size={16} />}
        title="Local AI server"
      >
        <LocalAiSettings actions={actions} settings={settings} />
      </SectionPanel>
      <div className="stack">
        <SectionPanel title="What it powers">
          <div className="feature-list">
            <div className="feature-row">
              <NotebookPen aria-hidden="true" size={16} />
              <span>
                <strong>Notes analysis</strong>
                <small>A summary and action items for each note, in History.</small>
              </span>
              {state}
            </div>
            <div className="feature-row">
              <Wand2 aria-hidden="true" size={16} />
              <span>
                <strong>Transform selection</strong>
                <small>
                  Rewrite highlighted text in any app with{" "}
                  {formatHotkey(settings.hotkeys.transformSelection)}.
                </small>
              </span>
              {state}
            </div>
          </div>
        </SectionPanel>
        {on ? (
          <TransformSelectionPanel
            shortcut={settings.hotkeys.transformSelection}
            showNotice={showNotice}
          />
        ) : null}
      </div>
    </section>
  );
}

export function GitHubBackupPage({ actions, settings }: PageProps) {
  return <SyncView actions={actions} part="github" settings={settings} />;
}

export function YourDataPage({ actions, settings }: PageProps) {
  return (
    <DataPrivacyView actions={actions} settings={settings}>
      <SyncView actions={actions} part="files" settings={settings} />
    </DataPrivacyView>
  );
}

export function AppearancePage({ actions, settings }: PageProps) {
  return (
    <div className="stack">
      <ThemesView actions={actions} settings={settings} />
      <section className="view-grid">
        <SectionPanel
          icon={<Palette aria-hidden="true" size={16} />}
          title="Pill colors"
        >
          <PillColorSettings actions={actions} settings={settings} />
        </SectionPanel>
        <WindowSizeSettings actions={actions} />
      </section>
    </div>
  );
}

export function GeneralPage({ actions, settings }: PageProps) {
  return (
    <SectionPanel
      icon={<SlidersHorizontal aria-hidden="true" size={16} />}
      title="App"
    >
      <AppBehaviorSettings actions={actions} settings={settings} />
    </SectionPanel>
  );
}
