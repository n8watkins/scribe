import { useEffect, useState } from "react";
import {
  getPastePermission,
  requestPastePermission,
  type AppSettings,
  type PastePermission,
} from "../backend";
import {
  afterDictationOf,
  formatHotkey,
  insertMethodOf,
  outputSettingsFor,
  type AfterDictation,
  type InsertMethod,
} from "../lib/format";
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
 * Windows clipboard format, and on Linux a hint that KDE's Klipper and other
 * managers honor. Nothing is claimed where it isn't done. */
function clipboardHistoryNote(): string {
  return forPlatform({
    windows: " Dictations stay out of Windows clipboard history.",
    macos: "",
    linux:
      " Dictations are marked so clipboard managers that honor it leave them out of their history.",
  });
}

/** The paste permission, read again whenever Scribe's window regains focus
 * (the user grants it in System Settings and comes back). */
function usePastePermission() {
  const [permission, setPermission] = useState<PastePermission | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = () => {
      getPastePermission()
        .then((next) => {
          if (active) setPermission(next);
        })
        .catch(() => undefined);
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => {
      active = false;
      window.removeEventListener("focus", refresh);
    };
  }, []);

  const request = () => {
    requestPastePermission()
      .then(setPermission)
      .catch(() => undefined);
  };

  return { permission, request };
}

/** The two output settings shared by Settings and Transcribe: what happens
 * after a dictation, and how inserted text reaches the app. Where pasting
 * needs a permission that is missing (Accessibility on macOS), a row to
 * grant it comes first. */
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
  const { permission, request } = usePastePermission();
  const afterDescriptions: Record<AfterDictation, string> = {
    insert: "Your words go straight into the app you're typing in.",
    copy: `Your words go on the clipboard for you to paste. ${pasteLast} also pastes the last one.`,
    save: `Your words are saved in History only. Paste the last one with ${pasteLast}.`,
  };

  return (
    <>
      {permission?.required && !permission.granted ? (
        <SettingRow
          description="macOS needs it before Scribe can paste or type into other apps. Until it's on, your words are copied for you to paste with Cmd+V. After a Scribe update you may need to turn it on again."
          label="Accessibility permission"
        >
          <button className="secondary-button" onClick={request} type="button">
            Open System Settings
          </button>
        </SettingRow>
      ) : null}
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
