import { type ReactNode } from "react";
import type { AppStateSnapshot } from "../backend";
import { stateTone } from "../lib/format";

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
