import { useEffect, useRef } from "react";
import { AlertTriangle } from "lucide-react";
import "./modal.css";

export function ConfirmDialog({
  busy = false,
  cancelLabel = "Cancel",
  confirmLabel = "Confirm",
  danger = false,
  message,
  onCancel,
  onConfirm,
  open,
  title,
}: {
  busy?: boolean;
  cancelLabel?: string;
  confirmLabel?: string;
  danger?: boolean;
  message: string;
  onCancel: () => void;
  onConfirm: () => void;
  open: boolean;
  title: string;
}) {
  const confirmRef = useRef<HTMLButtonElement>(null);

  // Close on Escape while open; move focus to the confirm button so the dialog
  // is keyboard-operable the moment it appears.
  useEffect(() => {
    if (!open) {
      return;
    }

    confirmRef.current?.focus();

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) {
        event.stopPropagation();
        onCancel();
      }
    };

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [busy, onCancel, open]);

  if (!open) {
    return null;
  }

  return (
    <div
      className="modal-overlay"
      onClick={(event) => {
        if (event.target === event.currentTarget && !busy) {
          onCancel();
        }
      }}
      role="presentation"
    >
      <div
        aria-modal="true"
        className={danger ? "modal-panel danger" : "modal-panel"}
        role="dialog"
      >
        <div className="modal-heading">
          <span className={danger ? "modal-icon danger" : "modal-icon"} aria-hidden="true">
            <AlertTriangle size={16} />
          </span>
          <h2>{title}</h2>
        </div>
        <p className="modal-message">{message}</p>
        <div className="modal-actions">
          <button
            className="ghost-button"
            disabled={busy}
            onClick={onCancel}
            type="button"
          >
            {cancelLabel}
          </button>
          <button
            className={danger ? "stop-button" : "primary-button"}
            disabled={busy}
            onClick={onConfirm}
            ref={confirmRef}
            type="button"
          >
            {busy ? "Working…" : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
