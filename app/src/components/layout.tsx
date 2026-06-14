import { type ReactNode } from "react";
import { type LucideIcon } from "lucide-react";
import type { AppSettings, BasicStats } from "../backend";
import {
  formatHotkey,
  formatNumber,
  formatOptionalDuration,
  formatOptionalNumber,
  hotkeyRows,
} from "../lib/format";

export function StatsCard({
  expanded = false,
  stats,
}: {
  expanded?: boolean;
  stats: BasicStats;
}) {
  const statRows = [
    { label: "Words today", value: formatNumber(stats.wordsToday) },
    { label: "Dictations today", value: formatNumber(stats.dictationsToday) },
    { label: "Average WPM", value: formatOptionalNumber(stats.averageWpm) },
    {
      label: "Latency avg",
      value: formatOptionalDuration(stats.averageTranscriptionLatencyMs),
    },
    {
      label: "Recording avg",
      value: formatOptionalDuration(stats.averageRecordingDurationMs),
    },
    { label: "Most used model", value: stats.mostUsedModel ?? "None" },
    {
      label: "Total words",
      value: formatNumber(stats.totalWordsTranscribed),
    },
  ];

  return (
    <div className={expanded ? "stats-grid wide" : "stats-grid"}>
      {statRows.map((stat) => (
        <div className="stat-tile" key={stat.label}>
          <span>{stat.label}</span>
          <strong title={stat.value}>{stat.value}</strong>
        </div>
      ))}
    </div>
  );
}

export function HotkeyList({
  compact = false,
  settings,
}: {
  compact?: boolean;
  settings: AppSettings;
}) {
  return (
    <div className={compact ? "hotkey-list compact-list" : "hotkey-list"}>
      {hotkeyRows(settings).map((hotkey) => (
        <div className="hotkey-row" key={hotkey.label}>
          <span>{hotkey.label}</span>
          <kbd>{formatHotkey(hotkey.value)}</kbd>
        </div>
      ))}
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
      <div className="card-header">
        <span>
          <Icon aria-hidden="true" size={13} />
          {label}
        </span>
        {status ?? null}
      </div>
      <div className="status-card-body">
        <strong title={value}>{value}</strong>
        <button className="ghost-button" onClick={onAction} type="button">
          {action}
        </button>
      </div>
    </article>
  );
}

export function SectionPanel({
  children,
  icon,
  title,
}: {
  children: ReactNode;
  icon?: ReactNode;
  title: string;
}) {
  return (
    <article className="panel-card">
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
}: {
  children: ReactNode;
  description: string;
  label: string;
}) {
  return (
    <div className="settings-row">
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
      <div className="setting-control">{children}</div>
    </div>
  );
}
