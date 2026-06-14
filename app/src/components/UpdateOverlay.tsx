import scribeIcon from "../assets/scribe-icon.png";
import "./update-overlay.css";

/** Drives the branded auto-update screen. App.tsx owns the actual
 * download/install flow and feeds progress in here, so this component is purely
 * presentational. */
export type UpdateOverlayState =
  | { phase: "preparing"; version?: string }
  | { phase: "downloading"; version?: string; percent: number | null }
  | { phase: "restarting"; version?: string }
  | { phase: "error"; version?: string; message: string };

/** A full-window, on-brand overlay shown while Scribe silently downloads and
 * installs an update on launch (instead of the native Windows installer UI).
 *
 * - preparing/downloading/restarting: a determinate (or indeterminate, when the
 *   server omits a content length) progress bar with status copy.
 * - error: a short message plus a "Continue" button that dismisses the overlay
 *   back into the app (the manual Install path in About still works).
 */
export function UpdateOverlay({
  state,
  onDismiss,
}: {
  state: UpdateOverlayState;
  /** Called from the error state's "Continue" button to return to the app. */
  onDismiss: () => void;
}) {
  const isError = state.phase === "error";
  const versionLabel = state.version ? `v${state.version}` : "";

  // The determinate bar reflects real download progress; when the phase is
  // "restarting" we pin it full, and when a content length is unknown we fall
  // back to an indeterminate (animated) bar.
  const percent =
    state.phase === "downloading"
      ? state.percent
      : state.phase === "restarting"
        ? 100
        : 0;
  const indeterminate =
    !isError &&
    (state.phase === "preparing" ||
      (state.phase === "downloading" && state.percent === null));

  const title = isError
    ? "Update paused"
    : state.phase === "restarting"
      ? "Restarting Scribe…"
      : "Updating Scribe…";

  const detail = (() => {
    switch (state.phase) {
      case "preparing":
        return "Getting the latest version ready…";
      case "downloading":
        return state.percent === null
          ? `Downloading update${versionLabel ? ` ${versionLabel}` : ""}…`
          : `Downloading update${versionLabel ? ` ${versionLabel}` : ""}… ${Math.round(
              state.percent,
            )}%`;
      case "restarting":
        return "Installing and restarting—this only takes a moment.";
      case "error":
        return state.message;
    }
  })();

  return (
    <div
      className="update-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <div className={`update-overlay-card${isError ? " is-error" : ""}`}>
        <img
          className="update-overlay-logo"
          src={scribeIcon}
          alt=""
          aria-hidden="true"
        />
        <h1 className="update-overlay-title">{title}</h1>
        <p className="update-overlay-detail">{detail}</p>

        {isError ? (
          <button
            className="primary-button update-overlay-continue"
            onClick={onDismiss}
            type="button"
          >
            Continue
          </button>
        ) : (
          <div
            className="update-overlay-progress"
            role="progressbar"
            aria-label="Update progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={
              indeterminate || percent === null ? undefined : Math.round(percent)
            }
          >
            <span
              className={`update-overlay-progress-fill${
                indeterminate ? " is-indeterminate" : ""
              }`}
              style={
                indeterminate
                  ? undefined
                  : { width: `${Math.max(0, Math.min(100, percent ?? 0))}%` }
              }
            />
          </div>
        )}

        {!isError ? (
          <p className="update-overlay-hint">
            Scribe will reopen automatically when it’s done.
          </p>
        ) : null}
      </div>
    </div>
  );
}
