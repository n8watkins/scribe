import { useState } from "react";
import { Check, ClipboardCopy, Upload } from "lucide-react";
import type { TextReplacement } from "../backend";
import {
  formatReplacementLines,
  missingFrom,
  parseReplacementLines,
} from "../lib/replacementSets";

/** Bulk paste in, with a preview of what will be added, and copy all out,
 * both as one "wrong -> right" per line. */
export function ReplacementsImport({
  current,
  disabled,
  onAdd,
}: {
  current: readonly TextReplacement[];
  disabled?: boolean;
  onAdd: (rows: TextReplacement[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [copied, setCopied] = useState(false);
  const parsed = parseReplacementLines(text);
  const fresh = missingFrom(current, parsed.pairs);
  const already = parsed.pairs.length - fresh.length;
  const exportText = formatReplacementLines(current);

  const copyAll = async () => {
    try {
      await navigator.clipboard.writeText(exportText);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // Clipboard can be refused; nothing else to do.
    }
  };

  return (
    <div className="replacement-import">
      <div className="row-actions">
        <button
          className="ghost-button"
          onClick={() => setOpen((value) => !value)}
          type="button"
        >
          <Upload aria-hidden="true" size={14} />
          Paste a list
        </button>
        <button
          className="ghost-button"
          disabled={!exportText}
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
      </div>
      {open ? (
        <div className="replacement-paste">
          <textarea
            aria-label="Replacements to add, one per line"
            className="vocab-textarea"
            onChange={(event) => setText(event.currentTarget.value)}
            placeholder={"nate builds -> n8builds\nsuperbase = Supabase\nclawed, Claude"}
            rows={4}
            value={text}
          />
          {text.trim() ? (
            <div className="replacement-preview" role="status">
              {fresh.length > 0 ? (
                <ul>
                  {fresh.map((row) => (
                    <li key={`${row.from}\u0000${row.to}`}>
                      {row.from} <span aria-hidden="true">→</span> {row.to}
                    </li>
                  ))}
                </ul>
              ) : null}
              <span className="replacement-preview-summary">
                {fresh.length} to add
                {already > 0 ? `, ${already} you already have` : ""}
                {parsed.errors.length > 0
                  ? `, ${parsed.errors.length} not understood`
                  : ""}
              </span>
              {parsed.errors.map((error) => (
                <span className="replacement-preview-error" key={error.line}>
                  Line {error.line}: "{error.text}" needs "wrong -&gt; right".
                </span>
              ))}
            </div>
          ) : null}
          <div className="row-actions">
            <button
              className="secondary-button"
              disabled={disabled || fresh.length === 0}
              onClick={() => {
                onAdd(fresh);
                setText("");
                setOpen(false);
              }}
              type="button"
            >
              Add {fresh.length || ""}
            </button>
            <button
              className="ghost-button"
              onClick={() => setOpen(false)}
              type="button"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
