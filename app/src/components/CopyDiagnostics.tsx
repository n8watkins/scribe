import { useEffect, useRef, useState } from "react";
import { Check, ClipboardCopy } from "lucide-react";
import { collectDiagnostics, commandErrorMessage } from "../backend";

/** About's "Copy diagnostics": copies a plain-text report (version, OS,
 * settings summary, recent problems and the end of the log) for a bug report.
 * The backend leaves out transcripts, tokens and personal paths. */
export function CopyDiagnostics() {
  const [state, setState] = useState<"idle" | "copying" | "copied">("idle");
  const [error, setError] = useState<string | null>(null);
  const resetTimer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(resetTimer.current), []);

  const copy = async () => {
    if (state === "copying") {
      return;
    }
    setState("copying");
    setError(null);
    try {
      const report = await collectDiagnostics();
      await navigator.clipboard.writeText(
        `${report.trimEnd()}\nWebView: ${navigator.userAgent}\n`,
      );
      setState("copied");
      window.clearTimeout(resetTimer.current);
      resetTimer.current = window.setTimeout(() => setState("idle"), 2500);
    } catch (cause) {
      setState("idle");
      setError(commandErrorMessage(cause));
    }
  };

  return (
    <div className="about-block" data-setting="Diagnostics">
      <div className="about-block-head">
        <strong>Diagnostics</strong>
      </div>
      <small>
        Copies a report to paste into a bug report: version, Windows version,
        a summary of your settings, recent errors and the end of the log. It
        never includes your transcripts, sign-in tokens or your user name.
      </small>
      <div className="about-block-actions">
        <button
          className="secondary-button about-update-button about-diagnostics-button"
          disabled={state === "copying"}
          onClick={() => void copy()}
          type="button"
        >
          {state === "copied" ? (
            <Check aria-hidden="true" size={15} />
          ) : (
            <ClipboardCopy aria-hidden="true" size={15} />
          )}
          {state === "copied"
            ? "Copied"
            : state === "copying"
              ? "Copying..."
              : "Copy diagnostics"}
        </button>
      </div>
      {error ? (
        <small className="about-diagnostics-error" role="alert">
          {error}
        </small>
      ) : null}
    </div>
  );
}
