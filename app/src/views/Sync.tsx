import { useCallback, useEffect, useRef, useState } from "react";
import {
  Cloud,
  CloudOff,
  Download,
  FolderSync,
  HardDriveUpload,
  RefreshCw,
} from "lucide-react";
import {
  commandErrorMessage,
  driveOrganizeNow,
  driveSyncNow,
  exportTranscripts,
  googleSignIn,
  googleSignOut,
  googleStatus,
  type AppSettings,
  type DriveSyncReport,
  type ExportFormat,
  type ExportScope,
  type GoogleStatus,
} from "../backend";
import type { ViewActions } from "../types";
import { SectionPanel, SettingRow } from "../components/layout";
import { Toggle } from "../components/primitives";
import "./sync.css";

/**
 * Sync is the home for backing up / exporting your notes and transcripts to
 * outside destinations. Today the only destination is Google Drive; the page is
 * laid out as a hub (Connection -> What to back up -> Organize / schedule) so
 * more targets can slot in later. The Google Drive logic here was promoted out
 * of Settings so backup is a first-class feature rather than a buried tab.
 */
export function SyncView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [status, setStatus] = useState<GoogleStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [lastReport, setLastReport] = useState<DriveSyncReport | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exportFormat, setExportFormat] = useState<ExportFormat>("markdown");
  const [exportScope, setExportScope] = useState<ExportScope>("all");
  const [exporting, setExporting] = useState(false);

  const reloadStatus = useCallback(async () => {
    try {
      const next = await googleStatus();
      setStatus(next);
    } catch (cause) {
      setError(commandErrorMessage(cause));
    }
  }, []);

  useEffect(() => {
    void reloadStatus();
  }, [reloadStatus]);

  // Source of truth is a stored token (status.signedIn), not the email — the
  // email can be blank on tokens granted before the email scope.
  const signedIn = status?.signedIn ?? false;
  // `configured === false` means this build ships without Drive credentials, so
  // sign-in can never succeed; surface that instead of a dead button.
  const unconfigured = status != null && !status.configured;

  const handleSignIn = useCallback(async () => {
    setBusy(true);
    setError(null);
    setNotice(null);

    try {
      const updated = await googleSignIn();
      actions.updateSettings(updated);
      await reloadStatus();
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [actions, reloadStatus]);

  const handleSignOut = useCallback(async () => {
    setBusy(true);
    setError(null);
    setNotice(null);
    setLastReport(null);

    try {
      const updated = await googleSignOut();
      actions.updateSettings(updated);
      await reloadStatus();
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [actions, reloadStatus]);

  const handleSyncNow = useCallback(async () => {
    setBusy(true);
    setError(null);
    setNotice(null);

    try {
      const report = await driveSyncNow();
      setLastReport(report);
      setNotice(
        `Synced ${report.syncedNotes} notes into ${report.filesWritten} file(s).`,
      );
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const handleOrganizeNow = useCallback(async () => {
    setBusy(true);
    setError(null);
    setNotice(null);

    try {
      const wrote = await driveOrganizeNow();
      setNotice(
        wrote
          ? "Organized today's notes into a Drive file."
          : "No notes for today yet — nothing to organize.",
      );
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const handleExport = useCallback(async () => {
    setExporting(true);
    setError(null);
    setNotice(null);

    try {
      const path = await exportTranscripts(exportScope, exportFormat);
      // A null path means the user cancelled the save dialog — stay quiet.
      if (path) {
        setNotice(`Exported to ${path}`);
      }
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setExporting(false);
    }
  }, [exportFormat, exportScope]);

  return (
    <section className="view-grid">
      <SectionPanel
        icon={<Cloud aria-hidden="true" size={16} />}
        title="Connection"
      >
        <div className="sync-destination-head">
          <span className="sync-destination-icon" aria-hidden="true">
            <HardDriveUpload size={18} />
          </span>
          <span className="sync-destination-meta">
            <strong>Google Drive</strong>
            <small>
              Back up and export your notes (and optionally transcripts) to a
              folder in your Google Drive.
            </small>
          </span>
          <span className="sync-destination-state">
            {unconfigured ? (
              <span className="pill error">
                <CloudOff aria-hidden="true" size={13} />
                Unavailable
              </span>
            ) : signedIn ? (
              <>
                <span className="status-dot success" aria-hidden="true" />
                <span className="pill ready">Connected</span>
              </>
            ) : (
              <span className="pill idle">Not connected</span>
            )}
          </span>
        </div>

        {unconfigured ? (
          <p className="muted vocab-hint">
            This build isn't configured for Google Drive sync.
          </p>
        ) : !signedIn ? (
          <>
            <p className="muted vocab-hint">
              Connect a Google account to back up your notes to Google Drive.
              Sign-in opens your browser to grant access.
            </p>
            <div className="setting-control">
              <button
                className="primary-button"
                disabled={busy}
                onClick={() => void handleSignIn()}
                type="button"
              >
                {busy ? "Signing in…" : "Sign in with Google"}
              </button>
            </div>
          </>
        ) : (
          <SettingRow
            description="The connected Google account that your backups are written to."
            label="Connected account"
          >
            <div className="setting-control">
              <span className="pill preserve">
                {settings.driveAccountEmail || status?.email || "Signed in"}
              </span>
              <button
                className="secondary-button"
                disabled={busy}
                onClick={() => void handleSignOut()}
                type="button"
              >
                {busy ? "Working…" : "Sign out"}
              </button>
            </div>
          </SettingRow>
        )}
      </SectionPanel>

      {signedIn && !unconfigured ? (
        <>
          <SectionPanel
            icon={<HardDriveUpload aria-hidden="true" size={16} />}
            title="What to back up"
          >
            <p className="muted vocab-hint">
              Choose what Scribe exports to Drive. Backups are written
              automatically as you go, and you can force one any time from
              Schedule below.
            </p>
            <SettingRow
              description="Write a Drive copy whenever a note is saved or analyzed."
              label="Sync notes to Drive"
            >
              <Toggle
                checked={settings.driveSyncEnabled}
                disabled={actions.savingSettings || busy}
                label="Sync notes to Drive"
                onChange={(driveSyncEnabled) =>
                  actions.updateSettings({ driveSyncEnabled })
                }
              />
            </SettingRow>
            <SettingRow
              description="Also back up every dictation transcript (not just notes) to a separate “Scribe Transcripts” folder in Drive, for a full-history backup."
              label="Back up all transcripts"
            >
              <Toggle
                checked={settings.driveSyncAllTranscripts}
                disabled={actions.savingSettings || busy}
                label="Back up all transcripts"
                onChange={(driveSyncAllTranscripts) =>
                  actions.updateSettings({ driveSyncAllTranscripts })
                }
              />
            </SettingRow>
          </SectionPanel>

          <SectionPanel
            icon={<FolderSync aria-hidden="true" size={16} />}
            title="Organize &amp; schedule"
          >
            <SettingRow
              description="At the hour below, the local LLM reorganizes the previous day's notes into a tidy Drive file."
              label="End-of-day auto-organize (local LLM)"
            >
              <Toggle
                checked={settings.driveOrganizeEnabled}
                disabled={actions.savingSettings || busy}
                label="End-of-day auto-organize (local LLM)"
                onChange={(driveOrganizeEnabled) =>
                  actions.updateSettings({ driveOrganizeEnabled })
                }
              />
            </SettingRow>
            <SettingRow
              description="Hour of day (local time) for the daily organize pass."
              label="Daily organize hour (local time)"
            >
              <select
                disabled={actions.savingSettings || busy}
                onChange={(event) =>
                  actions.updateSettings({
                    driveOrganizeHour: Number(event.currentTarget.value),
                  })
                }
                value={String(settings.driveOrganizeHour)}
              >
                {Array.from({ length: 24 }, (_, hour) => (
                  <option key={hour} value={hour}>
                    {String(hour).padStart(2, "0")}:00
                  </option>
                ))}
              </select>
            </SettingRow>
            <div className="sync-prompt-field">
              <span className="sync-prompt-label">
                <strong>Organize prompt</strong>
                <small>
                  The instruction sent to the local LLM when it reorganizes a
                  day's notes — it alone decides how the tidy Drive file reads.
                </small>
              </span>
              <SyncPromptTextArea
                ariaLabel="Drive organize prompt"
                disabled={actions.savingSettings}
                onSave={(driveOrganizePrompt) =>
                  actions.updateSettings({ driveOrganizePrompt })
                }
                placeholder="Reorganize today's dictated notes into a clean summary…"
                value={settings.driveOrganizePrompt}
              />
            </div>
            <p className="muted vocab-hint">
              Auto-organize needs the local LLM (notes analysis) running.
            </p>
            <div className="setting-control">
              <button
                className="secondary-button"
                disabled={busy}
                onClick={() => void handleSyncNow()}
                type="button"
              >
                <RefreshCw aria-hidden="true" size={15} />
                {busy ? "Syncing…" : "Sync now"}
              </button>
              <button
                className="secondary-button"
                disabled={busy}
                onClick={() => void handleOrganizeNow()}
                type="button"
              >
                <FolderSync aria-hidden="true" size={15} />
                {busy ? "Working…" : "Organize today now"}
              </button>
            </div>
          </SectionPanel>
        </>
      ) : null}

      <SectionPanel
        icon={<Download aria-hidden="true" size={16} />}
        title="Export to a file"
      >
        <p className="muted vocab-hint">
          Save your transcripts to a local file. No account needed — this writes
          straight to a folder you pick.
        </p>
        <SettingRow
          description="Markdown for reading, CSV for spreadsheets, JSON for the full raw data."
          label="Format"
        >
          <select
            disabled={exporting}
            onChange={(event) =>
              setExportFormat(event.currentTarget.value as ExportFormat)
            }
            value={exportFormat}
          >
            <option value="markdown">Markdown (.md)</option>
            <option value="csv">CSV (.csv)</option>
            <option value="json">JSON (.json)</option>
          </select>
        </SettingRow>
        <SettingRow
          description="Which transcripts to include in the export."
          label="What to export"
        >
          <select
            disabled={exporting}
            onChange={(event) =>
              setExportScope(event.currentTarget.value as ExportScope)
            }
            value={exportScope}
          >
            <option value="all">All transcripts</option>
            <option value="notes">Notes only</option>
            <option value="dictation">Dictation only</option>
          </select>
        </SettingRow>
        <div className="setting-control">
          <button
            className="secondary-button"
            disabled={exporting}
            onClick={() => void handleExport()}
            type="button"
          >
            <Download aria-hidden="true" size={15} />
            {exporting ? "Exporting…" : "Export…"}
          </button>
        </div>
      </SectionPanel>

      {notice ? <p className="muted vocab-hint sync-status-line">{notice}</p> : null}
      {!notice && lastReport ? (
        <p className="muted vocab-hint sync-status-line">
          Last sync: {lastReport.syncedNotes} notes, {lastReport.filesWritten}{" "}
          file(s).
        </p>
      ) : null}
      {error ? (
        <p className="muted vocab-hint sync-status-line" role="alert">
          {error}
        </p>
      ) : null}

      <p className="muted vocab-hint sync-more-note">
        More backup &amp; export destinations are coming. For now, everything
        syncs to Google Drive.
      </p>
    </section>
  );
}

/** Multi-line prompt field that mirrors the BlurSaved* pattern used elsewhere:
 * keystrokes live in a local draft and only flush to settings on blur (and on
 * unmount), so editing never round-trips the settings command per keystroke. */
function SyncPromptTextArea({
  ariaLabel,
  disabled,
  onSave,
  placeholder,
  value,
}: {
  ariaLabel: string;
  disabled?: boolean;
  onSave: (value: string) => void;
  placeholder?: string;
  value: string;
}) {
  const [draft, setDraft] = useState(value);
  const latestRef = useRef({ draft, onSave, value });
  latestRef.current = { draft, onSave, value };

  // Re-sync from props when the saved value changes elsewhere. Safe mid-edit:
  // we only flush on blur, so the prop doesn't move while the field is focused.
  useEffect(() => {
    setDraft(value);
  }, [value]);

  // Flush an unsaved draft if the view unmounts (e.g. sidebar navigation)
  // before blur fires, so a typed prompt is never lost.
  useEffect(
    () => () => {
      const latest = latestRef.current;
      if (latest.draft !== latest.value) {
        latest.onSave(latest.draft);
      }
    },
    [],
  );

  return (
    <textarea
      aria-label={ariaLabel}
      className="vocab-textarea"
      disabled={disabled}
      onBlur={() => {
        if (draft !== value) {
          onSave(draft);
        }
      }}
      onChange={(event) => setDraft(event.currentTarget.value)}
      placeholder={placeholder}
      rows={4}
      value={draft}
    />
  );
}
