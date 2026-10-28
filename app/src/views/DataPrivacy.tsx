import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  Check,
  ChevronDown,
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
  const [foldersOpen, setFoldersOpen] = useState(false);

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
    <section className="view-grid align-start">
      <SectionPanel
        icon={<ShieldCheck aria-hidden="true" size={16} />}
        title="History and audio clips"
      >
        <SettingRow
          description="Save each dictation on this computer so you can search it later in History."
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
          description="Delete dictations, their audio and failed recordings older than this. Notes keep their text, but their audio is deleted after this long too."
          label="Transcript retention"
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
          description="Delete saved notes older than this. Notes are kept forever by default."
          label="Notes retention"
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
          description="Keep each recording so you can play it back. Clips are deleted with their transcript, or after the transcript retention period for notes. Turn this off to stop saving new recordings."
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

      <div className="stack">
        <SectionPanel
          icon={<HardDrive aria-hidden="true" size={16} />}
          title="Local data"
        >
          {dataError ? (
            <InlineError message={dataError} onRetry={actions.refresh} />
          ) : null}
          <button
            aria-expanded={foldersOpen}
            className="accordion-toggle"
            onClick={() => setFoldersOpen((open) => !open)}
            type="button"
          >
            <span className="accordion-toggle-label">
              <FolderOpen aria-hidden="true" size={15} />
              Folders on this device
            </span>
            <span className="accordion-toggle-meta">
              <ChevronDown
                aria-hidden="true"
                className={foldersOpen ? "accordion-chevron is-open" : "accordion-chevron"}
                size={16}
              />
            </span>
          </button>

          {foldersOpen ? (
            <div className="local-data-accordion">
              <FolderRow
                label="Local data folder"
                note="Database and audio clips."
                onOpen={() => void handleOpenFolder(openDataFolder)}
                path={dataDir}
              />
              <hr className="local-data-divider" />
              <FolderRow
                label="Local models folder"
                note="Downloaded Whisper models."
                onOpen={() => void handleOpenFolder(openModelsFolder)}
                path={modelsDir}
              />
              <hr className="local-data-divider" />
              <FolderRow
                label="Local logs folder"
                note="Rotating diagnostic logs for bug reports."
                onOpen={() => void handleOpenFolder(openLogsFolder)}
                path={logsDir}
              />
              <hr className="local-data-divider" />
              <FolderRow
                label="Failed recordings folder"
                note="Audio kept when a transcription fails, so it isn't lost. Up to 20, deleted after the transcript retention period."
                onOpen={() => void handleOpenFolder(openFailedRecordingsFolder)}
                path={failedDir}
              />
            </div>
          ) : null}
        </SectionPanel>

        <SectionPanel
          icon={<Trash2 aria-hidden="true" size={16} />}
          title="Delete data"
        >
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
                : "Clear last transcript"}
            </button>
            <button
              className="ghost-button danger"
              disabled={clearingHistory}
              onClick={() => setConfirmClearHistory(true)}
              type="button"
            >
              <Trash2 aria-hidden="true" size={15} />
              {clearingHistory ? "Clearing..." : "Clear transcript history"}
            </button>
          </div>
        </SectionPanel>
      </div>

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

/** Saves the current window size as the default (Appearance). */
export function WindowSizeSettings({ actions }: { actions: ViewActions }) {
  const [dataError, setDataError] = useState<string | null>(null);
  const [savingWindowSize, setSavingWindowSize] = useState(false);
  const [windowSizeSaved, setWindowSizeSaved] = useState(false);

  const handleSaveWindowSize = useCallback(async () => {
    setDataError(null);
    setSavingWindowSize(true);
    setWindowSizeSaved(false);

    try {
      await saveWindowSize();
      await actions.refresh();
      setWindowSizeSaved(true);
      window.setTimeout(() => setWindowSizeSaved(false), 2500);
    } catch (error) {
      setDataError(commandErrorMessage(error));
    } finally {
      setSavingWindowSize(false);
    }
  }, [actions]);

  return (
    <>
      <SectionPanel
        icon={<Maximize2 aria-hidden="true" size={16} />}
        title="Window size"
      >
        <SettingRow
          description="Open Scribe at the size the window is now."
          label="Default window size"
        >
          <button
            className="secondary-button"
            disabled={savingWindowSize}
            onClick={() => void handleSaveWindowSize()}
            type="button"
          >
            <Maximize2 aria-hidden="true" size={15} />
            {savingWindowSize
              ? "Saving..."
              : windowSizeSaved
                ? "Saved"
                : "Save current size"}
          </button>
        </SettingRow>
      </SectionPanel>
      {dataError ? (
        <InlineError message={dataError} onRetry={actions.refresh} />
      ) : null}
    </>
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
    <div className="local-data-row">
      <span className="local-data-meta">
        <span className="local-data-label">{label}</span>
        <span className="local-data-note">{note}</span>
      </span>
      <code className="local-data-path" title={path ?? undefined}>
        {path ?? "Loading..."}
      </code>
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
