import { useMemo, useState, type ChangeEvent } from "react";
import {
  Check,
  ClipboardCopy,
  FileUp,
  FolderOpen,
  Plus,
  Search,
  X,
} from "lucide-react";
import {
  addReplacements,
  commandErrorMessage,
  openReplacementsFile,
  removeReplacement,
  type TextReplacement,
} from "../backend";
import type { AppSettings } from "../backend";
import type { ViewActions } from "../types";
import {
  DEVELOPER_REPLACEMENTS,
  formatReplacementLines,
  missingFrom,
  parseReplacementLines,
} from "../lib/replacementSets";
import { FORMAT_RULES } from "../lib/replacementFormat";
import { InfoDialog } from "./modal";
import { useReplacements } from "../lib/useReplacements";


/** Words > Replacements: a summary of replacements.txt, never the full list.
 * Add one, find and remove, paste or import a list, open the file, copy all. */
export function Replacements({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const { info, set } = useReplacements();
  const rows = useMemo(() => info?.rows ?? [], [info]);
  const [heard, setHeard] = useState("");
  const [written, setWritten] = useState("");
  const [query, setQuery] = useState("");
  const [importing, setImporting] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [showFormat, setShowFormat] = useState(false);

  const starter = missingFrom(rows, DEVELOPER_REPLACEMENTS);
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return rows;
    }
    return rows.filter(
      (row) =>
        row.from.toLowerCase().includes(needle) ||
        row.to.toLowerCase().includes(needle),
    );
  }, [query, rows]);

  const add = async (next: TextReplacement[]) => {
    setBusy(true);
    setError(null);
    try {
      const report = await addReplacements(next);
      set(report.info);
      setNotice(
        report.duplicates > 0
          ? `Added ${report.added}; ${report.duplicates} you already had.`
          : `Added ${report.added}.`,
      );
      return true;
    } catch (cause) {
      setError(commandErrorMessage(cause));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const remove = async (row: TextReplacement) => {
    setError(null);
    try {
      set(await removeReplacement(row));
    } catch (cause) {
      setError(commandErrorMessage(cause));
    }
  };

  const copyAll = async () => {
    try {
      await navigator.clipboard.writeText(formatReplacementLines(rows));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // Clipboard can be refused; nothing else to do.
    }
  };

  const readFile = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (file) {
      setImporting(await file.text());
    }
  };

  const parsed = importing !== null ? parseReplacementLines(importing) : null;
  const fresh = parsed ? missingFrom(rows, parsed.pairs) : [];

  return (
    <div className="replacements">
      <p className="muted vocab-hint">
        Say the left, get the right. No limit, and they work with every model.
      </p>

      <div className="replacements-summary">
        <span>
          <strong>{info ? rows.length : "..."}</strong>{" "}
          {rows.length === 1 ? "replacement" : "replacements"} in{" "}
          <span title={info?.path}>replacements.txt</span>
        </span>
        <div className="row-actions">
          <button
            className="ghost-button"
            onClick={() =>
              void openReplacementsFile().catch((cause) =>
                setError(commandErrorMessage(cause)),
              )
            }
            type="button"
          >
            <FolderOpen aria-hidden="true" size={14} />
            Open file
          </button>
          <button
            className="ghost-button"
            disabled={rows.length === 0}
            onClick={() => void copyAll()}
            type="button"
          >
            {copied ? (
              <Check aria-hidden="true" size={14} />
            ) : (
              <ClipboardCopy aria-hidden="true" size={14} />
            )}
            {copied ? "Copied" : "Copy all"}
          </button>
          <button
            aria-haspopup="dialog"
            className="setting-more"
            onClick={() => setShowFormat(true)}
            type="button"
          >
            Format
          </button>
        </div>
      </div>

      {info && info.skippedLines.length > 0 ? (
        <p className="hint-warning" role="alert">
          {info.skippedLines.length}{" "}
          {info.skippedLines.length === 1 ? "line" : "lines"} skipped:{" "}
          {info.skippedLines.join(", ")}. Each needs "heard -&gt; written".
        </p>
      ) : null}

      <InfoDialog
        onClose={() => setShowFormat(false)}
        open={showFormat}
        title="How to write replacements"
      >
        <ul className="replacements-format">
          {FORMAT_RULES.map((rule) => (
            <li key={rule}>{rule}</li>
          ))}
        </ul>
      </InfoDialog>

      {starter.length > 0 && !settings.replacementSuggestionDismissed ? (
        <div className="replacement-suggestion">
          <span>
            <strong>
              {starter.length} suggested{" "}
              {starter.length === 1 ? "fix" : "fixes"}
            </strong>{" "}
            for words speech models often mishear in developer talk:{" "}
            {starter
              .slice(0, 2)
              .map((row) => `${row.from} to ${row.to}`)
              .join(", ")}
            {starter.length > 2 ? " and more" : ""}.
          </span>
          <div className="row-actions">
            <button
              className="secondary-button"
              disabled={busy}
              onClick={() => void add(starter)}
              title={formatReplacementLines(starter)}
              type="button"
            >
              <Plus aria-hidden="true" size={14} />
              Add them
            </button>
            <button
              className="ghost-button"
              onClick={() =>
                actions.updateSettings({ replacementSuggestionDismissed: true })
              }
              type="button"
            >
              No thanks
            </button>
          </div>
        </div>
      ) : null}

      <form
        className="replacements-add"
        onSubmit={(event) => {
          event.preventDefault();
          if (!heard.trim() || !written.trim()) {
            return;
          }
          void add([{ from: heard.trim(), to: written.trim() }]).then((ok) => {
            if (ok) {
              setHeard("");
              setWritten("");
            }
          });
        }}
      >
        <input
          aria-label="Heard"
          onChange={(event) => setHeard(event.currentTarget.value)}
          placeholder="Heard"
          value={heard}
        />
        <span aria-hidden="true">→</span>
        <input
          aria-label="Written"
          onChange={(event) => setWritten(event.currentTarget.value)}
          placeholder="Written"
          value={written}
        />
        <button
          className="secondary-button"
          disabled={busy || !heard.trim() || !written.trim()}
          type="submit"
        >
          <Plus aria-hidden="true" size={14} />
          Add
        </button>
      </form>

      <label className="replacements-search">
        <Search aria-hidden="true" size={14} />
        <input
          aria-label="Find a replacement"
          onChange={(event) => setQuery(event.currentTarget.value)}
          placeholder="Find a replacement"
          value={query}
        />
      </label>
      <ul aria-label="Replacements" className="replacements-list">
        {shown.map((row) => (
          <li key={`${row.from}\u0000${row.to}`}>
            <span className="replacements-from">{row.from}</span>
            <span aria-hidden="true" className="replacements-arrow">
              →
            </span>
            <span className="replacements-to">{row.to}</span>
            <button
              aria-label={`Remove ${row.from}`}
              className="replacement-remove"
              onClick={() => void remove(row)}
              type="button"
            >
              <X aria-hidden="true" size={13} />
            </button>
          </li>
        ))}
        {info && shown.length === 0 ? (
          <li className="muted">{query.trim() ? "No match." : "None yet."}</li>
        ) : null}
      </ul>

      {importing === null ? (
        <div className="row-actions">
          <button
            className="ghost-button"
            onClick={() => setImporting("")}
            type="button"
          >
            Paste a list
          </button>
          <label className="ghost-button replacements-file">
            <FileUp aria-hidden="true" size={14} />
            Import file
            <input
              accept=".txt,.csv,.md,text/plain"
              onChange={(event) => void readFile(event)}
              type="file"
            />
          </label>
        </div>
      ) : (
        <div className="replacement-paste">
          <textarea
            aria-label="Replacements to add, one per line"
            className="vocab-textarea"
            onChange={(event) => setImporting(event.currentTarget.value)}
            placeholder={"nate builds -> n8builds\nsuperbase = Supabase\nclawed, Claude"}
            rows={4}
            value={importing}
          />
          {parsed && importing.trim() ? (
            <div className="replacement-preview" role="status">
              {fresh.length > 0 ? (
                <ul>
                  {fresh.slice(0, 50).map((row) => (
                    <li key={`${row.from}\u0000${row.to}`}>
                      {row.from} <span aria-hidden="true">→</span> {row.to}
                    </li>
                  ))}
                </ul>
              ) : null}
              <span className="replacement-preview-summary">
                {fresh.length} new
                {parsed.pairs.length - fresh.length > 0
                  ? `, ${parsed.pairs.length - fresh.length} already there`
                  : ""}
                {parsed.errors.length > 0
                  ? `, ${parsed.errors.length} not understood`
                  : ""}
              </span>
              {parsed.errors.slice(0, 5).map((lineError) => (
                <span className="replacement-preview-error" key={lineError.line}>
                  Line {lineError.line}: "{lineError.text}" needs "heard -&gt;
                  written".
                </span>
              ))}
            </div>
          ) : null}
          <div className="row-actions">
            <button
              className="secondary-button"
              disabled={busy || fresh.length === 0}
              onClick={() =>
                void add(fresh).then((ok) => {
                  if (ok) {
                    setImporting(null);
                  }
                })
              }
              type="button"
            >
              Add {fresh.length || ""}
            </button>
            <button
              className="ghost-button"
              onClick={() => setImporting(null)}
              type="button"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {notice ? (
        <p className="muted vocab-hint" role="status">
          {notice}
        </p>
      ) : null}
      {error ? (
        <p className="hint-warning" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
