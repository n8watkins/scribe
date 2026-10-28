import { type ReactNode } from "react";
import { AlertTriangle, ArrowUpRight, type LucideIcon } from "lucide-react";
import type { AppSettings, BasicStats } from "../backend";
import {
  formatHotkey,
  formatNumber,
  formatOptionalDuration,
  formatOptionalNumber,
  hotkeyRows,
} from "../lib/format";

/** Home's one-line summary of the local stats. The most used model is left
 * out: the Model card above already names the model in use. */
export function StatsRow({ stats }: { stats: BasicStats }) {
  const items = [
    { label: "Words today", value: formatNumber(stats.wordsToday) },
    { label: "Dictations today", value: formatNumber(stats.dictationsToday) },
    {
      label: "Speaking speed",
      value:
        stats.averageWpm === null
          ? "None"
          : `${formatOptionalNumber(stats.averageWpm)} wpm`,
    },
    {
      label: "Transcribe time",
      value: formatOptionalDuration(stats.averageTranscriptionLatencyMs),
    },
    {
      label: "Recording length",
      value: formatOptionalDuration(stats.averageRecordingDurationMs),
    },
    { label: "Total words", value: formatNumber(stats.totalWordsTranscribed) },
  ];

  return (
    <section
      aria-label="Your stats"
      className="stats-row"
      data-setting="Your stats"
      title="Averages and totals from the history on this device"
    >
      {items.map((item) => (
        <div className="stats-row-item" key={item.label}>
          <span>{item.label}</span>
          <strong>{item.value}</strong>
        </div>
      ))}
    </section>
  );
}

export function HotkeyList({
  compact = false,
  failedActions,
  only,
  settings,
}: {
  compact?: boolean;
  /** Actions whose shortcut failed to register (another app holds it). */
  failedActions?: ReadonlySet<string>;
  /** When set, list only these actions. */
  only?: ReadonlySet<string>;
  settings: AppSettings;
}) {
  return (
    <div className={compact ? "hotkey-list compact-list" : "hotkey-list"}>
      {hotkeyRows(settings)
        // Transform Selection is only bound while Local AI is on.
        .filter(
          (hotkey) =>
            (hotkey.action !== "transformSelection" ||
              settings.notesAnalysisEnabled) &&
            (!only || only.has(hotkey.action)),
        )
        .map((hotkey) => {
          const failed = failedActions?.has(hotkey.action) ?? false;
          return (
            <div
              className={failed ? "hotkey-row hotkey-row-failed" : "hotkey-row"}
              key={hotkey.label}
              title={
                failed
                  ? "Not registered. Another app may already use this shortcut."
                  : undefined
              }
            >
              <span>
                {failed ? (
                  <AlertTriangle aria-label="Not registered" role="img" size={13} />
                ) : null}
                {hotkey.label}
              </span>
              <kbd>{formatHotkey(hotkey.value)}</kbd>
            </div>
          );
        })}
    </div>
  );
}

export function StatusCard({
  action,
  Icon,
  label,
  onAction,
  status,
  value,
}: {
  action: string;
  Icon: LucideIcon;
  label: string;
  onAction: () => void;
  status?: ReactNode;
  value: string;
}) {
  return (
    <article className="metric-card status-card">
      <span className="status-card-icon" aria-hidden="true">
        <Icon size={17} />
      </span>
      <span className="status-card-main">
        <span className="status-card-label">{label}</span>
        <strong title={value}>{value}</strong>
      </span>
      {status ?? null}
      <button
        aria-label={action}
        className="status-card-go"
        onClick={onAction}
        title={action}
        type="button"
      >
        <ArrowUpRight aria-hidden="true" size={15} />
      </button>
    </article>
  );
}

export function SectionPanel({
  children,
  className,
  icon,
  title,
}: {
  children: ReactNode;
  /** Extra classes for the card, such as `span-2` to take a full grid row. */
  className?: string;
  icon?: ReactNode;
  title: string;
}) {
  return (
    <article
      className={className ? `panel-card ${className}` : "panel-card"}
      data-setting={title}
    >
      <div className="section-heading compact">
        <h2>
          {icon}
          {title}
        </h2>
      </div>
      <div className="settings-list">{children}</div>
    </article>
  );
}

export function SettingRow({
  children,
  description,
  label,
  stacked = false,
}: {
  children: ReactNode;
  description: string;
  label: string;
  /** Put a wide control (text field, text area) under the label. */
  stacked?: boolean;
}) {
  return (
    <div
      className={stacked ? "settings-row stacked" : "settings-row"}
      data-setting={label}
    >
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
      <div className="setting-control">{children}</div>
    </div>
  );
}
