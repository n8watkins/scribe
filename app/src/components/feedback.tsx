import { AlertCircle, CheckCircle2, Info, RefreshCw } from "lucide-react";
import type { ToastNotice } from "../types";

export function Toast({ notice }: { notice: ToastNotice }) {
  return (
    <div
      className={`toast-notice ${notice.tone}`}
      role={notice.tone === "error" ? "alert" : "status"}
    >
      {notice.tone === "error" ? (
        <AlertCircle aria-hidden="true" size={16} />
      ) : (
        <CheckCircle2 aria-hidden="true" size={16} />
      )}
      <span>{notice.message}</span>
    </div>
  );
}

export function LoadingPanel() {
  return (
    <article className="panel-card loading-panel">
      <RefreshCw aria-hidden="true" size={18} />
      <span>Loading dashboard data from Scribe commands...</span>
    </article>
  );
}

export function ErrorPanel({
  message,
  onRetry,
}: {
  message: string | null;
  onRetry: () => Promise<void>;
}) {
  return (
    <article className="panel-card error-panel">
      <AlertCircle aria-hidden="true" size={18} />
      <div>
        <strong>Could not load backend data</strong>
        <p>{message ?? "The Tauri command layer did not return data."}</p>
      </div>
      <button className="secondary-button" onClick={() => void onRetry()} type="button">
        <RefreshCw aria-hidden="true" size={15} />
        Retry
      </button>
    </article>
  );
}

export function InlineError({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => Promise<void>;
}) {
  return (
    <div className="inline-error">
      <AlertCircle aria-hidden="true" size={16} />
      <span>{message}</span>
      <button className="compact-action" onClick={() => void onRetry()} type="button">
        Refresh
      </button>
    </div>
  );
}

export function EmptyState({ message }: { message: string }) {
  return (
    <div className="empty-state">
      <Info aria-hidden="true" size={16} />
      <span>{message}</span>
    </div>
  );
}
