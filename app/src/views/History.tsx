import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  Cloud,
  Copy,
  Layers,
  ListChecks,
  NotebookPen,
  RefreshCw,
  Save,
  Search,
  Settings as SettingsIcon,
  SquareCheckBig,
  Trash2,
} from "lucide-react";
import {
  analyzeNote,
  clearNotes,
  clearTranscriptHistory,
  combineTranscripts,
  commandErrorMessage,
  copyTranscript,
  deleteTranscript,
  getTranscriptAudio,
  githubSyncNow,
  openTranscriptExternally,
  pasteTranscript,
  saveCombinedTranscript,
  searchTranscripts,
  type DashboardData,
  type Transcript,
  type TranscriptSort,
} from "../backend";
import type { ViewActions } from "../types";
import { EmptyState, InlineError } from "../components/feedback";
import { ConfirmDialog } from "../components/modal";
import { TranscriptRow } from "../components/transcript";
import "./history.css";

/// Converts a `YYYY-MM-DD` date-input value into the UTC instant for LOCAL
/// midnight `dayOffset` days later. The date picker reports a calendar day in
/// the user's own timezone, and the backend compares against `created_at` (a UTC
/// instant) with an EXCLUSIVE upper bound — so `from` uses offset 0 (start of
/// the selected local day) and `to` uses offset 1 (start of the next local day),
/// a clean half-open range that includes everything dictated on the local day.
function localDayBound(date: string, dayOffset: number): string | undefined {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!match) {
    return undefined;
  }
  const [, year, month, day] = match;
  return new Date(
    Number(year),
    Number(month) - 1,
    Number(day) + dayOffset,
  ).toISOString();
}

export function HistoryView({
  actions,
  data,
  notesOnly = false,
}: {
  actions: ViewActions;
  data: DashboardData;
  notesOnly?: boolean;
}) {
  const [query, setQuery] = useState("");
  const [fromDate, setFromDate] = useState("");
  const [toDate, setToDate] = useState("");
  const [sort, setSort] = useState<TranscriptSort>("newest");
  const [offset, setOffset] = useState(0);
  const [transcripts, setTranscripts] = useState<Transcript[]>([]);
  const [total, setTotal] = useState(0);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [busyTranscriptId, setBusyTranscriptId] = useState<string | null>(null);
  const [clearingHistory, setClearingHistory] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [syncingToGithub, setSyncingToGithub] = useState(false);
  const [syncNotice, setSyncNotice] = useState<string | null>(null);
  const [playingId, setPlayingId] = useState<string | null>(null);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [combinedText, setCombinedText] = useState<string | null>(null);
  const [combining, setCombining] = useState(false);
  const [savingCombined, setSavingCombined] = useState(false);
  const [combineCopied, setCombineCopied] = useState(false);
  const playbackRef = useRef<HTMLAudioElement | null>(null);
  const { settings } = data;
  const pageSize = 25;

  // Convert the From/To date inputs into the UTC instants the backend compares
  // against `created_at`. The picker is in the user's LOCAL timezone, so `from`
  // is local midnight of `fromDate` and `to` is local midnight of the day AFTER
  // `toDate` — the backend's EXCLUSIVE upper bound, a clean half-open range that
  // includes everything dictated on the local day. Each is omitted when empty.
  const fromBound = localDayBound(fromDate, 0);
  const toBound = localDayBound(toDate, 1);

  const loadHistory = useCallback(
    async (nextOffset: number) => {
      setHistoryLoading(true);
      setHistoryError(null);

      try {
        let result = await searchTranscripts({
          query: query.trim() || undefined,
          notesOnly: notesOnly || undefined,
          from: fromBound,
          to: toBound,
          sort,
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
            notesOnly: notesOnly || undefined,
            from: fromBound,
            to: toBound,
            sort,
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
    [query, notesOnly, fromBound, toBound, sort],
  );

  // Always points at the latest `loadHistory` so effects that must NOT re-run
  // when the query/filters change (and rebuild `loadHistory`) can still invoke
  // the current implementation without listing it as a dependency.
  const loadHistoryRef = useRef(loadHistory);
  loadHistoryRef.current = loadHistory;

  // The single trigger for query/filter changes: `loadHistory`'s identity
  // changes whenever query/from/to/sort change, so this debounced page-0 load
  // covers all of them (including each search keystroke) in one place.
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void loadHistory(0);
    }, 180);

    return () => window.clearTimeout(timer);
  }, [loadHistory]);

  // A changed filter reshuffles the result set, so any pending selection (and
  // its combine count) would be stale; clear it whenever a filter moves.
  useEffect(() => {
    setSelectedIds([]);
  }, [query, fromDate, toDate, sort, notesOnly]);

  // Reload when fresh data arrives (a new transcript/dictation) or the page
  // changes — but NOT when query/filters change: those flow through the
  // debounced page-0 effect above, so we call via the ref and omit
  // `loadHistory` from deps to avoid a second, undebounced per-keystroke query
  // (which also raced and could read a stale `offset`).
  useEffect(() => {
    void loadHistoryRef.current(offset);
  }, [data.lastTranscript?.id, data.stats.dictationsToday, offset]);

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
        // After a successful copy, drop this item from the selection (if it was
        // selected) so the checkbox state reflects that it's been handled.
        setSelectedIds((previous) => previous.filter((value) => value !== id));
        await refreshAfterMutation();
      } catch (error) {
        setHistoryError(commandErrorMessage(error));
      } finally {
        setBusyTranscriptId(null);
      }
    },
    [refreshAfterMutation],
  );

  const stopPlayback = useCallback(() => {
    playbackRef.current?.pause();
    playbackRef.current = null;
    setPlayingId(null);
  }, []);

  useEffect(() => stopPlayback, [stopPlayback]);

  const handlePlayTranscript = useCallback(
    async (id: string) => {
      if (playingId === id) {
        stopPlayback();
        return;
      }

      stopPlayback();
      setHistoryError(null);

      try {
        const base64 = await getTranscriptAudio(id);
        const audio = new Audio(`data:audio/wav;base64,${base64}`);
        // Reset on both natural end and decode/playback failure so a bad clip
        // doesn't strand the row showing "Stop" with no way to recover.
        const finish = () => {
          playbackRef.current = null;
          setPlayingId(null);
        };
        audio.onended = finish;
        audio.onerror = finish;
        playbackRef.current = audio;
        setPlayingId(id);
        await audio.play();
      } catch (error) {
        stopPlayback();
        setHistoryError(commandErrorMessage(error));
      }
    },
    [playingId, stopPlayback],
  );

  const handleAnalyzeNote = useCallback(async (id: string) => {
    setBusyTranscriptId(id);
    setHistoryError(null);

    try {
      const updated = await analyzeNote(id);
      // The row already holds the fresh analysis; no full reload needed.
      setTranscripts((previous) =>
        previous.map((item) => (item.id === updated.id ? updated : item)),
      );
    } catch (error) {
      setHistoryError(commandErrorMessage(error));
    } finally {
      setBusyTranscriptId(null);
    }
  }, []);

  const handleOpenExternally = useCallback(async (id: string) => {
    setHistoryError(null);
    try {
      await openTranscriptExternally(id);
    } catch (error) {
      setHistoryError(commandErrorMessage(error));
    }
  }, []);

  const toggleSelect = useCallback((id: string) => {
    setSelectedIds((previous) =>
      previous.includes(id)
        ? previous.filter((value) => value !== id)
        : [...previous, id],
    );
  }, []);

  // Whether every transcript currently on the page is selected, so the
  // select-all control can flip to "Deselect all" once nothing more can be
  // added on this page.
  const allVisibleSelected = useMemo(
    () =>
      transcripts.length > 0 &&
      transcripts.every((item) => selectedIds.includes(item.id)),
    [transcripts, selectedIds],
  );

  const toggleSelectAll = useCallback(() => {
    setSelectedIds((previous) => {
      const visibleIds = transcripts.map((item) => item.id);
      const everySelected =
        visibleIds.length > 0 &&
        visibleIds.every((id) => previous.includes(id));
      if (everySelected) {
        // Deselect just this page's items, leaving any off-page selection.
        const visible = new Set(visibleIds);
        return previous.filter((id) => !visible.has(id));
      }
      // Union the current selection with every visible id (no duplicates).
      return Array.from(new Set([...previous, ...visibleIds]));
    });
  }, [transcripts]);

  const closeCombine = useCallback(() => {
    setCombinedText(null);
    setCombineCopied(false);
  }, []);

  const handleCombine = useCallback(async () => {
    if (selectedIds.length < 2) {
      return;
    }

    setCombining(true);
    setHistoryError(null);
    setCombineCopied(false);

    try {
      const text = await combineTranscripts(selectedIds);
      setCombinedText(text);
    } catch (error) {
      setHistoryError(commandErrorMessage(error));
    } finally {
      setCombining(false);
    }
  }, [selectedIds]);

  const handleCopyCombined = useCallback(async () => {
    if (combinedText === null) {
      return;
    }
    try {
      await navigator.clipboard.writeText(combinedText);
      setCombineCopied(true);
      // The combined text is now on the clipboard, so deselect the items that
      // fed it — the selection has served its purpose for this copy.
      setSelectedIds([]);
    } catch (error) {
      setHistoryError(commandErrorMessage(error));
    }
  }, [combinedText]);

  const handleSaveCombined = useCallback(async () => {
    if (combinedText === null) {
      return;
    }

    setSavingCombined(true);
    setHistoryError(null);

    try {
      await saveCombinedTranscript(combinedText);
      setSelectedIds([]);
      closeCombine();
      await actions.refresh();
      await loadHistory(0);
    } catch (error) {
      setHistoryError(commandErrorMessage(error));
    } finally {
      setSavingCombined(false);
    }
  }, [actions, closeCombine, combinedText, loadHistory]);

  const handleDeleteTranscript = useCallback(
    async (id: string) => {
      if (!window.confirm("Delete this transcript from local history?")) {
        return;
      }

      setBusyTranscriptId(id);
      setHistoryError(null);

      try {
        await deleteTranscript(id);
        setSelectedIds((previous) => previous.filter((value) => value !== id));
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
    setClearingHistory(true);
    setHistoryError(null);

    try {
      // Notes view clears notes only; the transcript archive clears dictation
      // transcripts only (the backend preserves the other set either way).
      await (notesOnly ? clearNotes() : clearTranscriptHistory());
      setOffset(0);
      setSelectedIds([]);
      await refreshAfterMutation();
      setConfirmClear(false);
    } catch (error) {
      setHistoryError(commandErrorMessage(error));
    } finally {
      setClearingHistory(false);
    }
  }, [notesOnly, refreshAfterMutation]);

  const pageStart = total === 0 ? 0 : offset + 1;
  const pageEnd = Math.min(offset + pageSize, total);
  const hasPrevious = offset > 0;
  const hasNext = offset + pageSize < total;
  const selectedCount = useMemo(() => selectedIds.length, [selectedIds]);

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
          {notesOnly ? (
            settings.githubSyncEnabled ? (
              <button
                className="secondary-button"
                disabled={syncingToGithub}
                onClick={() => {
                  setSyncingToGithub(true);
                  setSyncNotice(null);
                  setHistoryError(null);
                  githubSyncNow()
                    .then((report) =>
                      setSyncNotice(
                        `Synced ${report.syncedNotes} note(s) to GitHub.`,
                      ),
                    )
                    .catch((cause) => setHistoryError(commandErrorMessage(cause)))
                    .finally(() => setSyncingToGithub(false));
                }}
                type="button"
              >
                <Cloud aria-hidden="true" size={15} />
                {syncingToGithub ? "Syncing…" : "Sync to GitHub"}
              </button>
            ) : null
          ) : (
            <button
              className="secondary-button danger"
              disabled={clearingHistory || total === 0}
              onClick={() => setConfirmClear(true)}
              type="button"
            >
              <Trash2 aria-hidden="true" size={15} />
              {clearingHistory ? "Clearing..." : "Clear all"}
            </button>
          )}
          <button
            className="ghost-button"
            onClick={() => actions.openSettings(notesOnly ? "notes" : "output")}
            type="button"
          >
            <SettingsIcon aria-hidden="true" size={15} />
            Settings
          </button>
        </div>
        <div className="toolbar-row filter-row">
          <label className="filter-field">
            <span className="filter-label">From</span>
            <input
              aria-label="Filter from date"
              max={toDate || undefined}
              onChange={(event) => setFromDate(event.currentTarget.value)}
              type="date"
              value={fromDate}
            />
          </label>
          <label className="filter-field">
            <span className="filter-label">To</span>
            <input
              aria-label="Filter to date"
              min={fromDate || undefined}
              onChange={(event) => setToDate(event.currentTarget.value)}
              type="date"
              value={toDate}
            />
          </label>
          <label className="filter-field">
            <span className="filter-label">Sort</span>
            <select
              aria-label="Sort transcripts"
              onChange={(event) =>
                setSort(event.currentTarget.value as TranscriptSort)
              }
              value={sort}
            >
              <option value="newest">Newest</option>
              <option value="oldest">Oldest</option>
              <option value="longest">Longest</option>
            </select>
          </label>
          <button
            className="secondary-button combine-button"
            disabled={selectedCount < 2 || combining}
            onClick={() => void handleCombine()}
            type="button"
          >
            <Layers aria-hidden="true" size={15} />
            {combining ? "Combining…" : `Combine (${selectedCount})`}
          </button>
        </div>
        {syncNotice ? (
          <p className="muted" style={{ margin: "8px 2px 0" }}>
            {syncNotice}
          </p>
        ) : null}
      </article>

      <article className="panel-card span-2">
        <div className="section-heading compact">
          {/* Select-all/deselect-all is ALWAYS visible (archive + notes), not
              only during an active selection. Disabled when the page is empty. */}
          <button
            className="select-all-button"
            disabled={transcripts.length === 0}
            onClick={toggleSelectAll}
            type="button"
          >
            {allVisibleSelected ? (
              <SquareCheckBig aria-hidden="true" size={14} />
            ) : (
              <ListChecks aria-hidden="true" size={14} />
            )}
            {allVisibleSelected ? "Deselect all" : "Select all"}
          </button>
          {notesOnly ? (
            <h2>
              <NotebookPen aria-hidden="true" size={16} />
              Notes
            </h2>
          ) : (
            <h2>
              <Archive aria-hidden="true" size={16} />
              Transcript archive
            </h2>
          )}
          <span className="notes-records">
            <span className="muted">
              {pageStart}-{pageEnd} of {total} local records
            </span>
            {notesOnly ? (
              <span className="notes-hint">
                <NotebookPen aria-hidden="true" size={12} />
                Dictate notes — hold ~ and tap Q
              </span>
            ) : null}
          </span>
        </div>
        {historyError ? (
          <InlineError message={historyError} onRetry={() => loadHistory(offset)} />
        ) : null}
        {!settings.historyEnabled ? (
          <EmptyState message="History is disabled. Existing records remain available until you delete them." />
        ) : null}
        {/* Only show the full-panel loading swap on the INITIAL load (no rows
            yet). A reload triggered by copy/insert/delete keeps the existing
            list mounted below, so the page's scroll position is preserved
            instead of collapsing to the top when the list briefly unmounts. */}
        {historyLoading && transcripts.length === 0 ? (
          <div className="pending-panel">
            <RefreshCw aria-hidden="true" size={16} />
            <span>Loading transcript history...</span>
          </div>
        ) : null}
        {!historyLoading && transcripts.length === 0 ? (
          <EmptyState
            message={
              notesOnly
                ? "No notes yet. Hold ~ and tap Q to dictate one."
                : "No local transcript records match this view yet."
            }
          />
        ) : null}
        {transcripts.length > 0 ? (
          <div className="transcript-list history-scroll">
            {transcripts.map((item) => (
              <TranscriptRow
                busy={busyTranscriptId === item.id}
                item={item}
                key={item.id}
                onAnalyze={
                  notesOnly && settings.notesAnalysisEnabled
                    ? handleAnalyzeNote
                    : undefined
                }
                onCopy={handleCopyTranscript}
                onDelete={handleDeleteTranscript}
                onOpenExternally={handleOpenExternally}
                onPaste={handlePasteTranscript}
                onPlay={handlePlayTranscript}
                onToggleSelect={toggleSelect}
                playing={playingId === item.id}
                selected={selectedIds.includes(item.id)}
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

      {combinedText !== null ? (
        <div
          className="combine-overlay"
          onClick={(event) => {
            if (event.target === event.currentTarget) {
              closeCombine();
            }
          }}
          role="presentation"
        >
          <div
            aria-label="Combined transcript preview"
            aria-modal="true"
            className="combine-panel"
            role="dialog"
          >
            <div className="section-heading compact">
              <h2>Combined transcript</h2>
              <span className="muted">{selectedCount} selected, oldest first</span>
            </div>
            <textarea
              className="combine-preview"
              readOnly
              value={combinedText}
            />
            <div className="button-row combine-actions">
              <button
                className="primary-button"
                disabled={savingCombined}
                onClick={() => void handleSaveCombined()}
                type="button"
              >
                <Save aria-hidden="true" size={15} />
                {savingCombined ? "Saving…" : "Save as new entry"}
              </button>
              <button
                className="secondary-button"
                disabled={savingCombined}
                onClick={() => void handleCopyCombined()}
                type="button"
              >
                <Copy aria-hidden="true" size={15} />
                {combineCopied ? "Copied" : "Copy"}
              </button>
              <button
                className="ghost-button"
                disabled={savingCombined}
                onClick={closeCombine}
                type="button"
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      ) : null}

      <ConfirmDialog
        busy={clearingHistory}
        confirmLabel={notesOnly ? "Clear notes" : "Clear history"}
        danger
        message={
          notesOnly
            ? "This permanently deletes every saved note (and its audio clip) on this device. Your dictation transcripts and Last Transcript Buffer are untouched. This can't be undone."
            : "This permanently deletes every saved dictation transcript (and its audio clip) on this device. Your notes and Last Transcript Buffer are untouched. This can't be undone."
        }
        onCancel={() => setConfirmClear(false)}
        onConfirm={() => void handleClearHistory()}
        open={confirmClear}
        title={notesOnly ? "Clear all notes?" : "Clear transcript history?"}
      />
    </section>
  );
}
