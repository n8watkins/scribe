import type { AppSettings } from "../backend";
import {
  afterDictationOf,
  formatHotkey,
  insertMethodOf,
  outputSettingsFor,
  type AfterDictation,
  type InsertMethod,
} from "../lib/format";
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
    "Scribe pastes the text, then puts back whatever was on your clipboard. Dictations stay out of Windows clipboard history.",
  paste_keep:
    "Scribe pastes the text and leaves it on your clipboard, replacing what was there, so you can paste it again.",
  type: "Scribe types the text out key by key. Use this for apps that block pasting, such as some terminals, remote desktops and games. Your clipboard is never touched.",
};

/** The two output settings shared by Settings and Transcribe: what happens
 * after a dictation, and how inserted text reaches the app. */
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
  const afterDescriptions: Record<AfterDictation, string> = {
    insert: "Your words go straight into the app you're typing in.",
    copy: `Your words go on the clipboard for you to paste. ${pasteLast} also pastes the last one.`,
    save: `Your words are saved in History only. Paste the last one with ${pasteLast}.`,
  };

  return (
    <>
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
        <SettingRow description={METHOD_DESCRIPTIONS[method]} label="Insert method">
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
