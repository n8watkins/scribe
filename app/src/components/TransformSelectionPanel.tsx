import { useState } from "react";
import { Wand2 } from "lucide-react";
import { commandErrorMessage, transformSelection } from "../backend";
import type { ToastNotice } from "../types";
import { formatHotkey } from "../lib/format";

/** Typed Transform selection: the user highlights text in another app, types
 * an instruction here, and Scribe hands focus back to that app, copies the
 * selection, rewrites it with the local AI, and pastes the result over it.
 * Shown on the Local AI page while Local AI is on. */
export function TransformSelectionPanel({
  shortcut,
  showNotice,
}: {
  shortcut: string;
  showNotice?: (message: string, tone?: ToastNotice["tone"]) => void;
}) {
  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    const trimmed = instruction.trim();
    if (!trimmed || busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // The other app keeps the highlighted selection; the backend refocuses
      // it, copies the selection, transforms it, and pastes the result back.
      const result = await transformSelection(trimmed);
      showNotice?.(result.message ?? "Selection transformed.", "success");
    } catch (caught) {
      const message = commandErrorMessage(caught);
      setError(message);
      showNotice?.(message, "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <article className="panel-card" data-setting="Transform selection">
      <div className="section-heading compact">
        <h2>
          <Wand2 aria-hidden="true" size={16} />
          Transform selection
        </h2>
        <kbd>{formatHotkey(shortcut)}</kbd>
      </div>
      <p className="muted transform-intro">
        Highlight text in any app, press {formatHotkey(shortcut)} and say the
        change. Or type it here: Scribe switches back to that app and rewrites
        the text in place.
      </p>
      <textarea
        aria-label="Transform instruction"
        className="transform-instruction"
        disabled={busy}
        onChange={(event) => setInstruction(event.target.value)}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault();
            void run();
          }
        }}
        placeholder="For example: make this shorter, fix the grammar, translate to Spanish"
        rows={2}
        value={instruction}
      />
      <div className="button-row">
        <button
          className="primary-button"
          disabled={busy || instruction.trim().length === 0}
          onClick={() => void run()}
          type="button"
        >
          <Wand2 aria-hidden="true" size={15} />
          {busy ? "Transforming..." : "Transform selection"}
        </button>
      </div>
      {error ? (
        <p className="muted" role="alert">
          {error}
        </p>
      ) : null}
    </article>
  );
}
