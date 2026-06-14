import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  Cloud,
  Layers,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import {
  analyzeNote,
  clearTranscriptHistory,
  combineTranscripts,
  commandErrorMessage,
  copyTranscript,
  deleteTranscript,
  driveSyncNow,
  getTranscriptAudio,
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
import { TranscriptRow } from "../components/transcript";

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
  const [syncingToDrive, setSyncingToDrive] = useState(false);
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

  // Convert the inclusive From/To date inputs into the RFC3339 bounds the
  // backend compares against `created_at` (chrono `to_rfc3339()`, +00:00).
  const fromBound = fromDate ? `${fromDate}T00:00:00+00:00` : undefined;
  const toBound = toDate ? `${toDate}T23:59:59.999+00:00` : undefined;

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
        audio.onended = () => {
          playbackRef.current = null;
          setPlayingId(null);
        };
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
    if (!window.confirm("Clear all saved transcript history?")) {
      return;
    }

    setClearingHistory(true);
    setHistoryError(null);

    try {
      await clearTranscriptHistory();
      setOffset(0);
      setSelectedIds([]);
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
            settings.driveSyncEnabled ? (
              <button
                className="secondary-button"
                disabled={syncingToDrive}
                onClick={() => {
                  setSyncingToDrive(true);
                  setSyncNotice(null);
                  setHistoryError(null);
                  driveSyncNow()
                    .then((report) =>
                      setSyncNotice(
                        `Synced ${report.syncedNotes} note(s) to Google Drive.`,
                      ),
                    )
                    .catch((cause) => setHistoryError(commandErrorMessage(cause)))
                    .finally(() => setSyncingToDrive(false));
                }}
                type="button"
              >
                <Cloud aria-hidden="true" size={15} />
                {syncingToDrive ? "Syncing…" : "Sync to Drive"}
              </button>
            ) : null
          ) : (
            <button
              className="secondary-button"
              disabled={clearingHistory || total === 0}
              onClick={() => void handleClearHistory()}
              type="button"
            >
              <Trash2 aria-hidden="true" size={15} />
              {clearingHistory ? "Clearing..." : "Clear all"}
            </button>
          )}
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
          <h2>{notesOnly ? "Notes" : "Transcript archive"}</h2>
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
          <EmptyState
            message={
              notesOnly
                ? "No notes yet. Hold ~ and tap Q to dictate one."
                : "No local transcript records match this view yet."
            }
          />
        ) : null}
        {!historyLoading && transcripts.length > 0 ? (
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
                {savingCombined ? "Saving…" : "Save as new entry"}
              </button>
              <button
                className="secondary-button"
                disabled={savingCombined}
                onClick={() => void handleCopyCombined()}
                type="button"
              >
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
    </section>
  );
}
