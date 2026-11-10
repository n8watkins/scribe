import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  Check,
  Copy,
  Eraser,
  FolderOpen,
  HardDrive,
  Maximize2,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import {
  clearTranscriptHistory,
  commandErrorMessage,
  getDataDir,
  getFailedRecordingsDir,
  getLogsDir,
  openDataFolder,
  openFailedRecordingsFolder,
  openLogsFolder,
  openModelsFolder,
  saveWindowSize,
  type AppSettings,
} from "../backend";
import type { ViewActions } from "../types";
import { retentionFromValue, retentionToValue } from "../lib/format";
import { SectionPanel, SettingRow } from "../components/layout";
import { Toggle } from "../components/primitives";
import { InlineError } from "../components/feedback";
import { ConfirmDialog } from "../components/modal";
import "./dataprivacy.css";

export function DataPrivacyView({
  actions,
  children,
  settings,
}: {
  actions: ViewActions;
  /** Extra panels (export and restore) placed before the delete section. */
  children?: ReactNode;
  settings: AppSettings;
}) {
  const [clearingHistory, setClearingHistory] = useState(false);
  const [confirmClearHistory, setConfirmClearHistory] = useState(false);
  const [dataError, setDataError] = useState<string | null>(null);
  const [dataDir, setDataDir] = useState<string | null>(null);
  const [logsDir, setLogsDir] = useState<string | null>(null);
  const [failedDir, setFailedDir] = useState<string | null>(null);

  // The models folder is the data dir's "models" subfolder (see the Rust
  // model_manager::models_dir). Derive it for display/copy using whichever
  // separator the platform path already uses.
  const modelsDir = dataDir
    ? `${dataDir.replace(/[\\/]+$/, "")}${dataDir.includes("\\") ? "\\" : "/"}models`
    : null;

  const loadDataDir = useCallback(async () => {
    try {
      setDataDir(await getDataDir());
    } catch (error) {
      setDataError(commandErrorMessage(error));
    }
    // The logs folder is the OS log dir (not under the data dir), so fetch it
    // separately. A failure here must not blank the whole Folders panel.
    try {
      setLogsDir(await getLogsDir());
    } catch {
      setLogsDir(null);
    }
    // The failed-recordings folder is the app-data dir's `failed/` subfolder
    // (where audio is kept when a transcription fails). Fetch it separately for
    // the same reason as the logs dir.
    try {
      setFailedDir(await getFailedRecordingsDir());
    } catch {
      setFailedDir(null);
    }
  }, []);

  useEffect(() => {
    void loadDataDir();
  }, [loadDataDir]);

  const handleClearHistory = useCallback(async () => {
    setClearingHistory(true);
    setDataError(null);

    try {
      await clearTranscriptHistory();
      await actions.refresh();
      setConfirmClearHistory(false);
    } catch (error) {
      setDataError(commandErrorMessage(error));
    } finally {
      setClearingHistory(false);
    }
  }, [actions]);

  const handleOpenFolder = useCallback(async (open: () => Promise<void>) => {
    setDataError(null);

    try {
      await open();
    } catch (error) {
      setDataError(commandErrorMessage(error));
    }
  }, []);

  return (
    <section className="view-grid data-grid">
      <SectionPanel
        icon={<ShieldCheck aria-hidden="true" size={16} />}
        title="History"
      >
        <SettingRow
          description="Save each dictation so you can search it in History."
          label="Keep history"
        >
          <Toggle
            checked={settings.historyEnabled}
            disabled={actions.savingSettings}
            label="Keep history"
            onChange={(historyEnabled) => actions.updateSettings({ historyEnabled })}
          />
        </SettingRow>
        <SettingRow
          description="Older dictations, their audio and failed recordings are deleted."
          label="Keep dictations for"
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
          description="Notes stay until you delete them, unless you set a limit."
          label="Keep notes for"
        >
          <select
            disabled={actions.savingSettings}
            onChange={(event) =>
              actions.updateSettings({
                notesRetentionDays: retentionFromValue(
                  event.currentTarget.value,
                ),
              })
            }
            value={retentionToValue(settings.notesRetentionDays)}
          >
            <option value="7">7 days</option>
            <option value="30">30 days</option>
            <option value="90">90 days</option>
            <option value="365">365 days</option>
            <option value="forever">Forever</option>
          </select>
        </SettingRow>
        <SettingRow
          description="Keep each recording so you can play it back. Clips go when their dictation does."
          label="Save audio clips"
        >
          <Toggle
            checked={settings.saveAudioClips}
            disabled={actions.savingSettings}
            label="Save audio clips"
            onChange={(saveAudioClips) => actions.updateSettings({ saveAudioClips })}
          />
        </SettingRow>
      </SectionPanel>

      <SectionPanel
        className="device-panel"
        icon={<HardDrive aria-hidden="true" size={16} />}
        title="On this device"
      >
        {dataError ? (
          <InlineError message={dataError} onRetry={actions.refresh} />
        ) : null}
        <div className="local-data-list">
          <FolderRow
            label="Data"
            note="Database and audio clips"
            onOpen={() => void handleOpenFolder(openDataFolder)}
            path={dataDir}
          />
          <FolderRow
            label="Models"
            note="Downloaded speech models"
            onOpen={() => void handleOpenFolder(openModelsFolder)}
            path={modelsDir}
          />
          <FolderRow
            label="Logs"
            note="For bug reports"
            onOpen={() => void handleOpenFolder(openLogsFolder)}
            path={logsDir}
          />
          <FolderRow
            label="Failed recordings"
            note="Up to 20, kept so you can retry"
            onOpen={() => void handleOpenFolder(openFailedRecordingsFolder)}
            path={failedDir}
          />
        </div>
        <div className="device-delete">
          <span className="device-delete-label">Delete</span>
          <div className="button-row data-delete-actions">
            <button
              className="secondary-button"
              disabled={actions.clearingLastTranscript}
              onClick={() => void actions.clearLastTranscript()}
              type="button"
            >
              <Eraser aria-hidden="true" size={15} />
              {actions.clearingLastTranscript
                ? "Clearing..."
                : "Last transcript"}
            </button>
            <button
              className="ghost-button danger"
              disabled={clearingHistory}
              onClick={() => setConfirmClearHistory(true)}
              type="button"
            >
              <Trash2 aria-hidden="true" size={15} />
              {clearingHistory ? "Clearing..." : "All history"}
            </button>
          </div>
        </div>
      </SectionPanel>

      {children}

      <ConfirmDialog
        busy={clearingHistory}
        confirmLabel="Clear history"
        danger
        message="This permanently deletes every saved dictation on this computer, with its audio, plus the last transcript, failed recordings and copies opened in an editor. Saved notes are kept. This can't be undone."
        onCancel={() => setConfirmClearHistory(false)}
        onConfirm={() => void handleClearHistory()}
        open={confirmClearHistory}
        title="Clear transcript history?"
      />

    </section>
  );
}

/** Window size presets, in logical pixels so they look the same at any
 * display scale. Standard is the size Scribe opens at on a new install. */
const WINDOW_PRESETS = [
  { id: "compact", label: "Compact", width: 900, height: 620 },
  { id: "standard", label: "Standard", width: 1024, height: 700 },
  { id: "large", label: "Large", width: 1240, height: 820 },
] as const;

/** Window size (Appearance): pick a preset, which resizes the window now and
 * keeps it as the size Scribe opens at, or keep the size you dragged it to. */
export function WindowSizeSettings({ actions }: { actions: ViewActions }) {
  const [dataError, setDataError] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  const [size, setSize] = useState({
    width: window.innerWidth,
    height: window.innerHeight,
  });

  useEffect(() => {
    const onResize = () =>
      setSize({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const activePreset = WINDOW_PRESETS.find(
    (preset) =>
      Math.abs(preset.width - size.width) <= 2 &&
      Math.abs(preset.height - size.height) <= 2,
  );

  const apply = useCallback(
    async (id: string, width?: number, height?: number) => {
      setDataError(null);
      setSaving(id);
      try {
        if (width && height) {
          await getCurrentWindow().setSize(new LogicalSize(width, height));
        }
        await saveWindowSize();
        await actions.refresh();
      } catch (error) {
        setDataError(commandErrorMessage(error));
      } finally {
        setSaving(null);
      }
    },
    [actions],
  );

  return (
    <SectionPanel
      icon={<Maximize2 aria-hidden="true" size={16} />}
      title="Window size"
    >
      <p className="muted vocab-hint">
        The size Scribe opens at. Now {size.width} x {size.height}.
      </p>
      <div className="window-presets" role="radiogroup" aria-label="Window size">
        {WINDOW_PRESETS.map((preset) => (
          <button
            aria-checked={activePreset?.id === preset.id}
            className={
              activePreset?.id === preset.id
                ? "window-preset is-active"
                : "window-preset"
            }
            disabled={saving !== null}
            key={preset.id}
            onClick={() => void apply(preset.id, preset.width, preset.height)}
            role="radio"
            type="button"
          >
            <strong>{preset.label}</strong>
            <small>
              {preset.width} x {preset.height}
            </small>
          </button>
        ))}
      </div>
      <button
        className="ghost-button"
        disabled={saving !== null || Boolean(activePreset)}
        onClick={() => void apply("current")}
        type="button"
      >
        <Maximize2 aria-hidden="true" size={14} />
        {saving === "current" ? "Saving..." : "Keep the current size"}
      </button>
      {dataError ? (
        <InlineError message={dataError} onRetry={actions.refresh} />
      ) : null}
    </SectionPanel>
  );
}

/** One folder entry in the "Local data" accordion: a label + note, the
 * monospace path, and a pair of small icon-buttons to open the folder and copy
 * its path. The copy button shows a brief check after a successful copy. */
function FolderRow({
  label,
  note,
  onOpen,
  path,
}: {
  label: string;
  note: string;
  onOpen: () => void;
  path: string | null;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    if (!path) {
      return;
    }
    try {
      await navigator.clipboard.writeText(path);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // Clipboard can be unavailable (e.g. denied permission); fail quietly.
    }
  }, [path]);

  return (
    <div className="local-data-row" title={path ?? undefined}>
      <span className="local-data-meta">
        <span className="local-data-label">{label}</span>
        <span className="local-data-note">{note}</span>
      </span>
      <span className="local-data-actions">
        <button
          aria-label={`Open ${label.toLowerCase()}`}
          className="icon-button"
          onClick={onOpen}
          type="button"
        >
          <FolderOpen aria-hidden="true" size={15} />
        </button>
        <button
          aria-label={copied ? "Path copied" : "Copy path"}
          className={copied ? "icon-button is-copied" : "icon-button"}
          disabled={!path}
          onClick={() => void handleCopy()}
          type="button"
        >
          {copied ? (
            <Check aria-hidden="true" size={15} />
          ) : (
            <Copy aria-hidden="true" size={15} />
          )}
        </button>
      </span>
    </div>
  );
}
