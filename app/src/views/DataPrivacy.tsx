import { useCallback, useEffect, useState } from "react";
import {
  Eraser,
  FolderOpen,
  Maximize2,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import {
  clearTranscriptHistory,
  commandErrorMessage,
  getDataDir,
  openDataFolder,
  openModelsFolder,
  saveWindowSize,
  type AppSettings,
} from "../backend";
import type { ViewActions } from "../types";
import { retentionFromValue, retentionToValue } from "../lib/format";
import { SectionPanel, SettingRow } from "../components/layout";
import { Toggle } from "../components/primitives";
import { InlineError } from "../components/feedback";

export function DataPrivacyView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [clearingHistory, setClearingHistory] = useState(false);
  const [dataError, setDataError] = useState<string | null>(null);
  const [dataDir, setDataDir] = useState<string | null>(null);
  const [savingWindowSize, setSavingWindowSize] = useState(false);
  const [windowSizeSaved, setWindowSizeSaved] = useState(false);

  const loadDataDir = useCallback(async () => {
    try {
      setDataDir(await getDataDir());
    } catch (error) {
      setDataError(commandErrorMessage(error));
    }
  }, []);

  useEffect(() => {
    void loadDataDir();
  }, [loadDataDir]);

  const handleClearHistory = useCallback(async () => {
    if (!window.confirm("Clear all saved transcript history?")) {
      return;
    }

    setClearingHistory(true);
    setDataError(null);

    try {
      await clearTranscriptHistory();
      await actions.refresh();
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
    <section className="view-grid">
      <SectionPanel
        icon={<ShieldCheck aria-hidden="true" size={16} />}
        title="History"
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
      </SectionPanel>

      <SectionPanel
        icon={<FolderOpen aria-hidden="true" size={16} />}
        title="Local data"
      >
        {dataError ? (
          <InlineError message={dataError} onRetry={actions.refresh} />
        ) : null}
        <div className="data-dir-field">
          <span className="data-dir-label">Data folder</span>
          <code className="data-dir-path" title={dataDir ?? undefined}>
            {dataDir ?? "Loading..."}
          </code>
          <p className="muted data-dir-note">
            Where Scribe keeps your database, audio clips, and models on this
            device.
          </p>
        </div>
        <div className="button-column">
          <button
            className="secondary-button"
            onClick={() => void handleOpenFolder(openDataFolder)}
            type="button"
          >
            <FolderOpen aria-hidden="true" size={15} />
            Open data folder
          </button>
          <button
            className="secondary-button"
            onClick={() => void handleOpenFolder(openModelsFolder)}
            type="button"
          >
            <FolderOpen aria-hidden="true" size={15} />
            Open models folder
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
          <button
            className="ghost-button danger"
            disabled={clearingHistory}
            onClick={() => void handleClearHistory()}
            type="button"
          >
            <Trash2 aria-hidden="true" size={15} />
            {clearingHistory ? "Clearing..." : "Clear transcript history"}
          </button>
        </div>
      </SectionPanel>

      <SectionPanel
        icon={<Maximize2 aria-hidden="true" size={16} />}
        title="Window"
      >
        <SettingRow
          description="Reopen Scribe at the window's current size."
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
                : "Save current size as default"}
          </button>
        </SettingRow>
      </SectionPanel>
    </section>
  );
}
