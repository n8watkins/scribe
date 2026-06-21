import { useCallback, useEffect, useRef, useState } from "react";
import { Cloud, CloudOff, Download, GitBranch, RefreshCw } from "lucide-react";
import {
  commandErrorMessage,
  exportTranscripts,
  githubDevicePoll,
  githubDeviceStart,
  githubDisconnect,
  githubStatus,
  githubSyncNow,
  type AppSettings,
  type ExportFormat,
  type ExportScope,
  type GithubDeviceCode,
  type GithubStatus,
  type GithubSyncReport,
} from "../backend";
import type { ViewActions } from "../types";
import { SectionPanel, SettingRow } from "../components/layout";
import { Toggle } from "../components/primitives";
import "./sync.css";

/**
 * Sync is the home for backing up / exporting your notes and transcripts to
 * outside destinations. Today the only destination is GitHub (a private repo of
 * dated Markdown); the page is laid out as a hub (Connection -> What to back up)
 * so more targets can slot in later.
 */
export function SyncView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [status, setStatus] = useState<GithubStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [device, setDevice] = useState<GithubDeviceCode | null>(null);
  const [lastReport, setLastReport] = useState<GithubSyncReport | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exportFormat, setExportFormat] = useState<ExportFormat>("markdown");
  const [exportScope, setExportScope] = useState<ExportScope>("all");
  const [exporting, setExporting] = useState(false);

  const reloadStatus = useCallback(async () => {
    try {
      const next = await githubStatus();
      setStatus(next);
    } catch (cause) {
      setError(commandErrorMessage(cause));
    }
  }, []);

  useEffect(() => {
    void reloadStatus();
  }, [reloadStatus]);

  // Source of truth is a stored token (status.connected).
  const connected = status?.connected ?? false;
  // `configured === false` means this build ships without a GitHub OAuth client
  // id, so connecting can never succeed; surface that instead of a dead button.
  const unconfigured = status != null && !status.configured;
  // The sync toggles require a VALID "owner/name" repo, or the save is rejected
  // (validate()) and the first sync just errors. A non-empty but malformed value
  // (e.g. a bare "notes") must not arm the toggles.
  const repoValid = /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/.test(
    settings.githubRepo.trim(),
  );

  // A monotonic epoch for connect attempts. handleConnect snapshots the current
  // epoch at its start; Cancel (and any new attempt) bumps it. A late-resolving
  // poll (the backend keeps polling until the device code expires; the request
  // can't be aborted mid-flight) then no-ops because its snapshot is stale — so
  // a cancel→reconnect race can't let a stale poll clobber the new attempt.
  const connectEpoch = useRef(0);

  // Two-step device flow: get + show the code, then await authorization.
  const handleConnect = useCallback(async () => {
    const myEpoch = ++connectEpoch.current;
    setBusy(true);
    setError(null);
    setNotice(null);

    try {
      const code = await githubDeviceStart();
      if (connectEpoch.current !== myEpoch) {
        return;
      }
      // Show the code immediately, THEN block on the (long-running) poll.
      setDevice(code);
      const updated = await githubDevicePoll(code.deviceCode, code.intervalSecs);
      if (connectEpoch.current !== myEpoch) {
        return;
      }
      actions.updateSettings(updated);
      await reloadStatus();
    } catch (cause) {
      if (connectEpoch.current === myEpoch) {
        setError(commandErrorMessage(cause));
      }
    } finally {
      if (connectEpoch.current === myEpoch) {
        setDevice(null);
        setBusy(false);
      }
    }
  }, [actions, reloadStatus]);

  // Back out of an in-progress sign-in. The orphaned backend poll keeps running
  // until GitHub's device code expires (~15 min) and then stops on its own;
  // bumping the epoch makes its eventual result a no-op.
  const handleCancelConnect = useCallback(() => {
    connectEpoch.current += 1;
    setDevice(null);
    setBusy(false);
    setNotice("GitHub sign-in cancelled.");
  }, []);

  const handleDisconnect = useCallback(async () => {
    setBusy(true);
    setError(null);
    setNotice(null);
    setLastReport(null);

    try {
      const updated = await githubDisconnect();
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
      const report = await githubSyncNow();
      setLastReport(report);
      setNotice(
        `Synced ${report.syncedNotes} notes into ${report.filesWritten} file(s).`,
      );
    } catch (cause) {
      setError(commandErrorMessage(cause));
      // A github_unauthorized error has already cleared the dead token in the
      // backend; refresh status so the Connected pill flips to Not connected.
      void reloadStatus();
    } finally {
      setBusy(false);
    }
  }, [reloadStatus]);

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
            <GitBranch size={18} />
          </span>
          <span className="sync-destination-meta">
            <strong>GitHub</strong>
            <small>
              Back up your notes as dated Markdown to a private GitHub repo.
            </small>
          </span>
          <span className="sync-destination-state">
            {unconfigured ? (
              <span className="pill error">
                <CloudOff aria-hidden="true" size={13} />
                Unavailable
              </span>
            ) : connected ? (
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
            This build isn't configured for GitHub sync.
          </p>
        ) : device != null ? (
          <div className="setting-control" style={{ flexDirection: "column", alignItems: "flex-start" }}>
            <p className="muted vocab-hint">
              Go to <strong>{device.verificationUri}</strong> and enter this
              code:
            </p>
            <span className="pill preserve">{device.userCode}</span>
            <p className="muted vocab-hint">Waiting for authorization…</p>
            <button
              className="secondary-button"
              onClick={handleCancelConnect}
              type="button"
            >
              Cancel
            </button>
          </div>
        ) : !connected ? (
          <>
            <p className="muted vocab-hint">
              Connect a GitHub account to back up your notes. We'll open GitHub
              in your browser to enter a one-time code. GitHub will ask for
              access to your repositories — Scribe uses it only to create and
              write your private backup repo.
            </p>
            <div className="setting-control">
              <button
                className="primary-button"
                disabled={busy}
                onClick={() => void handleConnect()}
                type="button"
              >
                <GitBranch aria-hidden="true" size={15} />
                {busy ? "Connecting…" : "Connect GitHub"}
              </button>
            </div>
          </>
        ) : (
          <SettingRow
            description="The connected GitHub account that your backups are written with."
            label="Connected account"
          >
            <div className="setting-control">
              <span className="pill preserve">
                {settings.githubAccountLogin || status?.username || "Connected"}
              </span>
              <button
                className="secondary-button"
                disabled={busy}
                onClick={() => void handleDisconnect()}
                type="button"
              >
                {busy ? "Working…" : "Disconnect"}
              </button>
            </div>
          </SettingRow>
        )}
      </SectionPanel>

      {connected && !unconfigured ? (
        <SectionPanel
          icon={<GitBranch aria-hidden="true" size={16} />}
          title="What to back up"
        >
          <p className="muted vocab-hint">
            Choose what Scribe writes to GitHub. Backups are written
            automatically as you go, and you can force one any time.
          </p>
          <SettingRow
            description="The private repo to write notes to, as owner/name (e.g. alice/scribe-notes). Created automatically if it's yours and missing."
            label="Repository"
          >
            <RepoInput
              disabled={actions.savingSettings || busy}
              onSave={(githubRepo) => {
                // Clearing the repo must also disable sync, or validate()
                // rejects an empty-repo-with-sync-on save (and we'd keep
                // pushing to nowhere). A non-empty value saves as-is.
                const trimmed = githubRepo.trim();
                actions.updateSettings(
                  trimmed.length === 0
                    ? {
                        githubRepo: "",
                        githubSyncEnabled: false,
                        githubSyncAllTranscripts: false,
                      }
                    : { githubRepo: trimmed },
                );
              }}
              value={settings.githubRepo}
            />
          </SettingRow>
          <SettingRow
            description={
              repoValid
                ? "Write a GitHub copy whenever a note is saved or analyzed."
                : "Set a valid owner/name repository above first."
            }
            label="Sync notes to GitHub"
          >
            <Toggle
              checked={settings.githubSyncEnabled}
              disabled={actions.savingSettings || busy || !repoValid}
              label="Sync notes to GitHub"
              onChange={(githubSyncEnabled) =>
                actions.updateSettings({ githubSyncEnabled })
              }
            />
          </SettingRow>
          <SettingRow
            description={
              repoValid
                ? "Also back up every dictation transcript (not just notes) to a separate “transcripts” folder in the repo, for a full-history backup."
                : "Set a valid owner/name repository above first."
            }
            label="Back up all transcripts"
          >
            <Toggle
              checked={settings.githubSyncAllTranscripts}
              disabled={actions.savingSettings || busy || !repoValid}
              label="Back up all transcripts"
              onChange={(githubSyncAllTranscripts) =>
                actions.updateSettings({ githubSyncAllTranscripts })
              }
            />
          </SettingRow>
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
          </div>
        </SectionPanel>
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
        syncs to GitHub.
      </p>
    </section>
  );
}

/** Single-line repo input that mirrors the BlurSaved* pattern used elsewhere:
 * keystrokes live in a local draft and only flush to settings on blur (and on
 * unmount), so editing never round-trips the settings command per keystroke. */
function RepoInput({
  disabled,
  onSave,
  value,
}: {
  disabled?: boolean;
  onSave: (value: string) => void;
  value: string;
}) {
  const [draft, setDraft] = useState(value);

  // Re-sync from props when the saved value changes elsewhere. Safe mid-edit:
  // we only flush on blur, so the prop doesn't move while the field is focused.
  useEffect(() => {
    setDraft(value);
  }, [value]);

  return (
    <input
      aria-label="GitHub repository"
      disabled={disabled}
      onBlur={() => {
        const trimmed = draft.trim();
        if (trimmed !== value) {
          onSave(trimmed);
        }
      }}
      onChange={(event) => setDraft(event.currentTarget.value)}
      placeholder="owner/scribe-notes"
      type="text"
      value={draft}
    />
  );
}
