import type { AppSettings, PastePermission } from "../backend";
import {
  afterDictationOf,
  formatHotkey,
  insertMethodOf,
  outputSettingsFor,
  type AfterDictation,
  type InsertMethod,
} from "../lib/format";
import { usePastePermission } from "../lib/pastePermission";
import { forPlatform } from "../lib/platform";
import { SettingRow } from "./layout";
import { SegmentedControl } from "./primitives";

const AFTER_OPTIONS: { label: string; value: AfterDictation }[] = [
  { label: "Insert at cursor", value: "insert" },
  { label: "Copy only", value: "copy" },
  { label: "Save only", value: "save" },
];

const METHOD_OPTIONS: { label: string; value: InsertMethod }[] = [
  { label: "Paste, keeps your clipboard", value: "paste_restore" },
  { label: "Paste, leaves it on the clipboard", value: "paste_keep" },
  { label: "Type it out", value: "type" },
];

const METHOD_DESCRIPTIONS: Record<InsertMethod, string> = {
  paste_restore:
    "Scribe pastes the text, then puts back whatever was on your clipboard.",
  paste_keep:
    "Scribe pastes the text and leaves it on your clipboard, replacing what was there, so you can paste it again.",
  type: "Scribe types the text out key by key. Use this for apps that block pasting, such as some terminals, remote desktops and games. Your clipboard is never touched.",
};

/** What keeps a dictation out of clipboard history differs per platform: a
 * Windows clipboard format, on Linux a hint that KDE's Klipper and other
 * managers honor, and on macOS the nspasteboard.org concealed type. */
function clipboardHistoryNote(): string {
  return forPlatform({
    windows: " Dictations stay out of Windows clipboard history.",
    macos:
      " Dictations are marked so clipboard managers that honor it leave them out of their history.",
    linux:
      " Dictations are marked so clipboard managers that honor it leave them out of their history.",
  });
}

/** The row that grants a missing paste permission: Accessibility on macOS,
 * the desktop's one-time remote-control dialog on Wayland. */
function PermissionRow({
  permission,
  request,
  requesting,
}: {
  permission: PastePermission | null;
  request: () => void;
  requesting: boolean;
}) {
  if (!permission?.required || permission.granted) {
    return null;
  }
  if (permission.kind === "waylandPortal") {
    return (
      <SettingRow
        description={
          permission.declined
            ? "You turned this down, so Scribe copies your words and you press Ctrl+V. Allow it and Scribe pastes for you."
            : "Your desktop asks once whether Scribe may press Shift+Insert for you in Wayland apps, and Scribe remembers the answer. Without it, your first dictation asks."
        }
        label="Paste into Wayland apps"
      >
        <button
          className="secondary-button"
          disabled={requesting}
          onClick={request}
          type="button"
        >
          {requesting
            ? "Waiting for your desktop..."
            : permission.declined
              ? "Ask again"
              : "Allow pasting"}
        </button>
      </SettingRow>
    );
  }
  return (
    <SettingRow
      description="macOS needs it before Scribe can paste or type into other apps. Until it's on, your words are copied for you to paste with Cmd+V. After a Scribe update you may need to turn it on again."
      label="Accessibility permission"
    >
      <button className="secondary-button" onClick={request} type="button">
        Open System Settings
      </button>
    </SettingRow>
  );
}

/** The two output settings shared by Settings and Transcribe: what happens
 * after a dictation, and how inserted text reaches the app. Where pasting
 * needs a permission that is missing (Accessibility on macOS, the desktop's
 * remote-control portal on Wayland), a row to grant it comes first. */
export function OutputSettingRows({
  disabled,
  onChange,
  settings,
}: {
  disabled?: boolean;
  onChange: (patch: Partial<AppSettings>) => void;
  settings: AppSettings;
}) {
  const after = afterDictationOf(settings.outputMode);
  const method = insertMethodOf(settings);
  const pasteLast = formatHotkey(settings.hotkeys.pasteLastTranscript);
  const { permission, request, requesting } = usePastePermission();
  const afterDescriptions: Record<AfterDictation, string> = {
    insert: "Your words go straight into the app you're typing in.",
    copy: `Your words go on the clipboard for you to paste. ${pasteLast} also pastes the last one.`,
    save: `Your words are saved in History only. Paste the last one with ${pasteLast}.`,
  };

  return (
    <>
      <PermissionRow
        permission={permission}
        request={request}
        requesting={requesting}
      />
      <SettingRow description={afterDescriptions[after]} label="After dictation">
        <SegmentedControl
          className="output-after-control"
          disabled={disabled}
          onChange={(next) => onChange(outputSettingsFor(next, method))}
          options={AFTER_OPTIONS}
          selected={after}
        />
      </SettingRow>
      {after === "insert" ? (
        <SettingRow
          description={
            method === "paste_restore"
              ? `${METHOD_DESCRIPTIONS[method]}${clipboardHistoryNote()}`
              : METHOD_DESCRIPTIONS[method]
          }
          label="Insert method"
        >
          <select
            aria-label="Insert method"
            className="output-method-select"
            disabled={disabled}
            onChange={(event) =>
              onChange(
                outputSettingsFor("insert", event.currentTarget.value as InsertMethod),
              )
            }
            value={method}
          >
            {METHOD_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </SettingRow>
      ) : null}
    </>
  );
}
