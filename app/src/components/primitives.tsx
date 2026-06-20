import { useEffect, useState, type ReactNode } from "react";
import type { AppStateSnapshot } from "../backend";
import { formatMsReadable, stateTone } from "../lib/format";

export function Toggle({
  checked,
  disabled = false,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      aria-label={label}
      aria-pressed={checked}
      className={checked ? "toggle is-on" : "toggle"}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      type="button"
    >
      <span />
    </button>
  );
}

export function IconButton({
  children,
  danger = false,
  disabled = false,
  label,
  onClick,
}: {
  children: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  label: string;
  onClick?: () => void;
}) {
  return (
    <button
      aria-label={label}
      className={danger ? "icon-button danger" : "icon-button"}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      {children}
    </button>
  );
}

/** A digit-only millisecond field (no spinner) that shows a live "≈ Ns"
 * read-out. It keeps the keystrokes in local state and only commits on blur or
 * Enter, so typing never re-renders the rest of the settings column. */
export function MsInput({
  ariaLabel,
  disabled = false,
  min = 1,
  max,
  onCommit,
  value,
}: {
  ariaLabel: string;
  disabled?: boolean;
  min?: number;
  max?: number;
  onCommit: (ms: number) => void;
  value: number;
}) {
  const [text, setText] = useState(String(value));

  // Re-sync from props when the committed value changes elsewhere (reset to
  // defaults, a backend clamp). Safe mid-edit: we only commit on blur, so the
  // prop doesn't change while typing.
  useEffect(() => {
    setText(String(value));
  }, [value]);

  const commit = () => {
    const parsed = Number(text);
    if (text.trim() === "" || !Number.isFinite(parsed)) {
      setText(String(value));
      return;
    }
    // Clamp to [min, max] so the field can never commit a value the backend
    // would reject (e.g. a segment cap above Whisper's safe window).
    let next = Math.max(min, Math.round(parsed));
    if (max !== undefined) {
      next = Math.min(max, next);
    }
    setText(String(next));
    if (next !== value) {
      onCommit(next);
    }
  };

  return (
    <div className="duration-field">
      <input
        aria-label={ariaLabel}
        disabled={disabled}
        inputMode="numeric"
        onBlur={commit}
        onChange={(event) =>
          setText(event.currentTarget.value.replace(/[^0-9]/g, ""))
        }
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.currentTarget.blur();
          }
        }}
        type="text"
        value={text}
      />
      <small className="muted">≈ {formatMsReadable(Number(text) || 0)}</small>
    </div>
  );
}

export function SegmentedControl<T extends string>({
  disabled = false,
  onChange,
  options,
  selected,
}: {
  disabled?: boolean;
  onChange: (selected: T) => void;
  options: { label: string; value: T }[];
  selected: T;
}) {
  return (
    <div className="segmented-control">
      {options.map((option) => (
        <button
          aria-pressed={option.value === selected}
          className={option.value === selected ? "active-segment" : ""}
          disabled={disabled}
          key={option.value}
          onClick={() => onChange(option.value)}
          type="button"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function StatePill({ appState }: { appState: AppStateSnapshot }) {
  const className = `pill ${stateTone(appState.status)}`;
  const label = appState.error?.message ?? appState.status;

  return (
    <span className={className} title={label}>
      {appState.status}
    </span>
  );
}
