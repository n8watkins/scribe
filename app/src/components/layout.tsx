import { useState, type ReactNode } from "react";
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
      label: "Recording time",
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
      {items.map((item) => {
        // "148 wpm" -> a big 148 and a small unit, so the numbers carry the row.
        const match = /^([\d.,]+)\s*([a-z]+)$/i.exec(item.value);
        return (
          <div className="stats-row-item" key={item.label}>
            <span>{item.label}</span>
            <strong>
              {match ? (
                <>
                  {match[1]}
                  <small className="stats-row-unit">{match[2]}</small>
                </>
              ) : (
                item.value
              )}
            </strong>
          </div>
        );
      })}
    </section>
  );
}

export function HotkeyList({
  compact = false,
  failedActions,
  only,
  settings,
  systemTriggers,
}: {
  compact?: boolean;
  /** Actions whose shortcut failed to register (another app holds it). */
  failedActions?: ReadonlySet<string>;
  /** When set, list only these actions. */
  only?: ReadonlySet<string>;
  settings: AppSettings;
  /** When the desktop owns the hotkeys (Wayland portal): the keys it
   * assigned, by action, shown instead of Scribe's own bindings. */
  systemTriggers?: Readonly<Record<string, string>>;
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
          const keys = systemTriggers
            ? (systemTriggers[hotkey.action] ?? "Not set")
            : formatHotkey(hotkey.value);
          return (
            <div
              className={failed ? "hotkey-row hotkey-row-failed" : "hotkey-row"}
              key={hotkey.label}
              title={
                failed
                  ? systemTriggers
                    ? "Your desktop hasn't assigned keys to this yet."
                    : "Not registered. Another app may already use this shortcut."
                  : undefined
              }
            >
              <span>
                {failed ? (
                  <AlertTriangle aria-label="Not registered" role="img" size={13} />
                ) : null}
                {hotkey.label}
              </span>
              <kbd>{keys}</kbd>
            </div>
          );
        })}
    </div>
  );
}

export function StatusCard({
  action,
  detail,
  Icon,
  label,
  onAction,
  status,
  value,
}: {
  action: string;
  /** A short live line under the value. */
  detail?: string;
  Icon: LucideIcon;
  label: string;
  onAction: () => void;
  status?: ReactNode;
  value: string;
}) {
  // The whole card is the button: a bigger target than the old corner arrow,
  // and one less control competing for the card's width.
  return (
    <button
      className="metric-card status-card"
      onClick={onAction}
      title={action}
      type="button"
    >
      <span className="status-card-icon" aria-hidden="true">
        <Icon size={16} />
      </span>
      <span className="status-card-main">
        <span className="status-card-label">
          {label}
          {status ?? null}
        </span>
        <strong title={value}>{value}</strong>
        {detail ? (
          <span className="status-card-detail" role="status" title={detail}>
            {detail}
          </span>
        ) : null}
      </span>
      <ArrowUpRight aria-hidden="true" className="status-card-go" size={14} />
    </button>
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
  more,
  blockedBy,
  stacked = false,
}: {
  children: ReactNode;
  /** One plain line. */
  description: string;
  label: string;
  /** Details behind a small "More", for the few who need them. */
  more?: string;
  /** Another setting turns this one off or makes it do nothing: the reason,
   * and a link to that setting when it lives on another page. */
  blockedBy?: Blocked | null;
  /** Put a wide control (text field, text area) under the label. */
  stacked?: boolean;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div
      className={stacked ? "settings-row stacked" : "settings-row"}
      data-setting={label}
    >
      <span>
        <strong>{label}</strong>
        <small>
          {description}
          {more ? (
            open ? (
              <span className="setting-more-text"> {more}</span>
            ) : (
              <>
                {" "}
                <button
                  aria-label={`More about ${label}`}
                  className="setting-more"
                  onClick={() => setOpen(true)}
                  type="button"
                >
                  More
                </button>
              </>
            )
          ) : null}
        </small>
        {blockedBy ? <BlockedNote blocked={blockedBy} /> : null}
      </span>
      <div className="setting-control">{children}</div>
    </div>
  );
}

/** Why a setting is off or does nothing, and where to change that. */
export type Blocked = {
  reason: string;
  open?: { label: string; onClick: () => void };
};

/** The "Off because ..." line under a setting another one controls. */
export function BlockedNote({ blocked }: { blocked: Blocked }) {
  return (
    <span className="setting-blocked" role="note">
      {blocked.reason}
      {blocked.open ? (
        <>
          {" "}
          <button
            className="setting-more"
            onClick={blocked.open.onClick}
            type="button"
          >
            {blocked.open.label}
          </button>
        </>
      ) : null}
    </span>
  );
}
