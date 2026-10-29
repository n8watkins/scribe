import { useCallback, useEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import {
  commandErrorMessage,
  LANGUAGE_AUTO,
  listModels,
  llmStatus,
  SUPPORTED_LANGUAGES,
  type AppSettings,
  type LlmStatus,
  type ModelInfo,
  type TextReplacement,
} from "../backend";
import type { ViewActions } from "../types";
import { SettingRow } from "../components/layout";
import { MsInput, Toggle } from "../components/primitives";
import { currentPlatform } from "../lib/platform";

// Groups of setting rows. The sidebar pages in views/pages.tsx place them:
// Dictation, Words, Models & language, Local AI, Appearance and General.

type GroupProps = { actions: ViewActions; settings: AppSettings };

/** Spoken language and translation, shown next to the model that decides
 * whether they work. */
export function LanguageSettings({ actions, settings }: GroupProps) {
  // The selected model's multilingual capability drives the notice shown when
  // a non-English language (or translation) is chosen with an English-only
  // model. Null while the catalog loads, so the notice never flashes for the
  // default English setup.
  const [selectedModelMultilingual, setSelectedModelMultilingual] = useState<
    boolean | null
  >(null);
  useEffect(() => {
    let cancelled = false;
    void listModels()
      .then((models: ModelInfo[]) => {
        if (cancelled) {
          return;
        }
        const active = models.find(
          (model) => model.selected || model.id === settings.selectedModelId,
        );
        setSelectedModelMultilingual(active ? active.multilingual : null);
      })
      .catch(() => {
        // A catalog load failure shouldn't surface here; just skip the notice.
        if (!cancelled) {
          setSelectedModelMultilingual(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [settings.selectedModelId]);

  // The user asked for non-English work (a specific non-English language,
  // auto-detect, or translation) but the active model is English-only.
  const wantsMultilingual =
    settings.translateToEnglish ||
    settings.language === LANGUAGE_AUTO ||
    (settings.language !== "en" && settings.language.length > 0);
  const showMultilingualNotice =
    wantsMultilingual && selectedModelMultilingual === false;

  return (
    <>
      <SettingRow
        description="The language you speak. Auto-detect and any language other than English need a multilingual model."
        label="Language"
      >
        <select
          aria-label="Language"
          disabled={actions.savingSettings}
          onChange={(event) =>
            actions.updateSettings({ language: event.currentTarget.value })
          }
          value={settings.language}
        >
          <option value="auto">Auto-detect</option>
          {SUPPORTED_LANGUAGES.map(([code, label]) => (
            <option key={code} value={code}>
              {label}
            </option>
          ))}
        </select>
      </SettingRow>
      <SettingRow
        description="Speak any language and get the text in English. Needs a multilingual model."
        label="Translate to English"
      >
        <Toggle
          checked={settings.translateToEnglish}
          disabled={actions.savingSettings}
          label="Translate to English"
          onChange={(translateToEnglish) =>
            actions.updateSettings({ translateToEnglish })
          }
        />
      </SettingRow>
      {showMultilingualNotice ? (
        <p className="muted vocab-hint" role="status">
          Your model only understands English, so this won't take effect.
          Download and choose a multilingual model above, such as Base or Small
          Multilingual.
        </p>
      ) : null}
    </>
  );
}

/** Filler word removal, which only removes a filler when the speaker paused
 * around it. */
export function FillerSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <SettingRow
        description={
          'Remove words like "um" and "uh", but only when you pause around them, so fluent uses ("oh no", "like this") stay. Runs on this computer.'
        }
        label="Remove filler words"
      >
        <Toggle
          checked={settings.fillerSuppressionEnabled}
          disabled={actions.savingSettings}
          label="Remove filler words"
          onChange={(fillerSuppressionEnabled) =>
            actions.updateSettings({ fillerSuppressionEnabled })
          }
        />
      </SettingRow>
      <SettingRow
        description="The words to remove, separated by commas. Riskier ones such as oh, like and so are safe to add because of the pause check."
        label="Filler words"
        stacked
      >
        <BlurSavedTextArea
          ariaLabel="Filler words, comma-separated"
          onSave={(raw) =>
            actions.updateSettings({ fillerWords: parseFillerWords(raw) })
          }
          placeholder="um, uh, er, hmm"
          rows={2}
          value={settings.fillerWords.join(", ")}
        />
      </SettingRow>
      <details className="advanced-disclosure">
        <summary>Advanced</summary>
        <div className="settings-list">
          <SettingRow
            description="How long the pause next to a filler word must be before it is removed. Lower removes more (100 to 1500 ms)."
            label="Pause before removing (ms)"
          >
            <MsInput
              ariaLabel="Pause length before removing a filler, in milliseconds"
              disabled={
                actions.savingSettings || !settings.fillerSuppressionEnabled
              }
              max={1500}
              min={100}
              onCommit={(fillerPauseThresholdMs) =>
                actions.updateSettings({ fillerPauseThresholdMs })
              }
              value={settings.fillerPauseThresholdMs}
            />
          </SettingRow>
        </div>
      </details>
    </>
  );
}

/** Whether the floating pill shows, and what it shows. */
export function PillSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <SettingRow
        description="A small bar that stays on top of other windows and shows when Scribe is listening."
        label="Show floating pill"
      >
        <Toggle
          checked={settings.showFloatingPill}
          disabled={actions.savingSettings}
          label="Show floating pill"
          onChange={(showFloatingPill) =>
            actions.updateSettings({ showFloatingPill })
          }
        />
      </SettingRow>
      <SettingRow
        description="What the pill shows while you dictate."
        label="Pill style"
      >
        <select
          aria-label="Pill style"
          disabled={actions.savingSettings}
          onChange={(event) =>
            actions.updateSettings({
              pillDisplayMode:
                event.currentTarget.value === "dot"
                  ? "dot"
                  : event.currentTarget.value === "visualizer"
                    ? "visualizer"
                    : "visualizer_with_text",
            })
          }
          value={settings.pillDisplayMode}
        >
          <option value="dot">Dot</option>
          <option value="visualizer">Waveform</option>
          <option value="visualizer_with_text">Waveform and text</option>
        </select>
      </SettingRow>
    </>
  );
}

/** The floating pill's colors (Appearance). */
export function PillColorSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <SettingRow
        description="Waveform and dot color while you dictate."
        label="Pill color"
      >
        <input
          aria-label="Pill color"
          disabled={actions.savingSettings}
          onChange={(event) =>
            actions.updateSettings({ pillColorNormal: event.currentTarget.value })
          }
          type="color"
          value={settings.pillColorNormal}
        />
      </SettingRow>
      <SettingRow
        description="Waveform and dot color while you take a note."
        label="Note pill color"
      >
        <input
          aria-label="Note pill color"
          disabled={actions.savingSettings}
          onChange={(event) =>
            actions.updateSettings({ pillColorNote: event.currentTarget.value })
          }
          type="color"
          value={settings.pillColorNote}
        />
      </SettingRow>
      <SettingRow
        description="Background color of the pill."
        label="Pill background"
      >
        <input
          aria-label="Pill background color"
          disabled={actions.savingSettings}
          onChange={(event) =>
            actions.updateSettings({
              pillColorBackground: event.currentTarget.value,
            })
          }
          type="color"
          value={settings.pillColorBackground}
        />
      </SettingRow>
    </>
  );
}

/** Startup, tray, notifications, sounds and developer mode (General). */
export function AppBehaviorSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <SettingRow
        description={
          currentPlatform() === "windows"
            ? "Start Scribe when you sign in to Windows."
            : "Start Scribe when you log in."
        }
        label="Launch at startup"
      >
        <Toggle
          checked={settings.launchAtStartup}
          disabled={actions.savingSettings}
          label="Launch at startup"
          onChange={(launchAtStartup) => actions.updateSettings({ launchAtStartup })}
        />
      </SettingRow>
      <SettingRow
        description="Keep Scribe available from the system tray."
        label="Minimize to tray"
      >
        <Toggle
          checked={settings.minimizeToTray}
          disabled={actions.savingSettings}
          label="Minimize to tray"
          onChange={(minimizeToTray) => actions.updateSettings({ minimizeToTray })}
        />
      </SettingRow>
      <SettingRow
        description="Pressing the Open Scribe hotkey while this window is in front hides it again."
        label="Open Scribe hotkey hides it"
      >
        <Toggle
          checked={settings.dashboardHotkeyToggles}
          disabled={actions.savingSettings}
          label="Open Scribe hotkey hides it"
          onChange={(dashboardHotkeyToggles) =>
            actions.updateSettings({ dashboardHotkeyToggles })
          }
        />
      </SettingRow>
      <SettingRow
        description="Show a short message when a dictation finishes or fails."
        label="Notifications"
      >
        <Toggle
          checked={settings.notificationsEnabled}
          disabled={actions.savingSettings}
          label="Notifications"
          onChange={(notificationsEnabled) =>
            actions.updateSettings({ notificationsEnabled })
          }
        />
      </SettingRow>
      <SettingRow
        description="Play a tone when recording starts and stops."
        label="Sounds"
      >
        <Toggle
          checked={settings.soundsEnabled}
          disabled={actions.savingSettings}
          label="Sounds"
          onChange={(soundsEnabled) => actions.updateSettings({ soundsEnabled })}
        />
      </SettingRow>
      <SettingRow
        description="Add a Developer page to the sidebar with diagnostics such as the window size."
        label="Developer settings"
      >
        <Toggle
          checked={settings.developerSettingsEnabled}
          disabled={actions.savingSettings}
          label="Developer settings"
          onChange={(developerSettingsEnabled) =>
            actions.updateSettings({ developerSettingsEnabled })
          }
        />
      </SettingRow>
    </>
  );
}

/** Names and words the speech model should expect. */
export function ContextHintSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <p className="muted vocab-hint">
        Names, jargon and spellings Scribe should expect, for example "Tauri,
        natkins, whisper.cpp". This nudges recognition toward them but can't
        guarantee an exact result; for that, use Replacements.
      </p>
      <BlurSavedTextArea
        ariaLabel="Context hint"
        onSave={(vocabularyPrompt) => actions.updateSettings({ vocabularyPrompt })}
        placeholder="Tauri, natkins, whisper.cpp"
        value={settings.vocabularyPrompt}
      />
    </>
  );
}

/** "Say this, write that" replacements. */
export function ReplacementsSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <p className="muted vocab-hint">
        Whenever you say the phrase on the left, Scribe writes the text on the
        right. For example, "my email" can become your address, or "clawed"
        can become "Claude". Case doesn't matter, and only whole words match.
      </p>
      <ReplacementsEditor
        disabled={actions.savingSettings}
        onChange={(textReplacements) => actions.updateSettings({ textReplacements })}
        value={settings.textReplacements}
      />
    </>
  );
}

/** The one Local AI switch, the server connection, and the analysis prompt. */
export function LocalAiSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <SettingRow
        description="Turns on Notes analysis and Transform selection, and their hotkey. Text goes only to the server below; with a local address it never leaves this computer."
        label="Use local AI"
      >
        <Toggle
          checked={settings.notesAnalysisEnabled}
          disabled={actions.savingSettings}
          label="Use local AI"
          onChange={(notesAnalysisEnabled) =>
            actions.updateSettings({ notesAnalysisEnabled })
          }
        />
      </SettingRow>
      <SettingRow
        description="Where your AI server (LM Studio, Ollama or another OpenAI-compatible server) listens. LM Studio uses http://127.0.0.1:1234/v1."
        label="Server address"
        stacked
      >
        <BlurSavedInput
          ariaLabel="Local AI server address"
          onSave={(notesAnalysisEndpoint) =>
            actions.updateSettings({ notesAnalysisEndpoint })
          }
          placeholder="http://127.0.0.1:1234/v1"
          value={settings.notesAnalysisEndpoint}
        />
      </SettingRow>
      <SettingRow
        description="Leave empty to use whichever model the server has loaded."
        label="Model"
        stacked
      >
        <BlurSavedInput
          ariaLabel="Local AI model"
          onSave={(notesAnalysisModel) =>
            actions.updateSettings({ notesAnalysisModel })
          }
          placeholder="Loaded model (automatic)"
          value={settings.notesAnalysisModel}
        />
      </SettingRow>
      <SettingRow
        description="Sent with each note when you analyze it, so it decides what you get back (a summary, action items, tags)."
        label="Analysis prompt"
        stacked
      >
        <BlurSavedTextArea
          ariaLabel="Notes analysis prompt"
          onSave={(notesAnalysisPrompt) =>
            actions.updateSettings({ notesAnalysisPrompt })
          }
          placeholder="Summarize this dictated note..."
          rows={4}
          value={settings.notesAnalysisPrompt}
        />
      </SettingRow>
      <LlmConnectionCard
        endpoint={settings.notesAnalysisEndpoint}
        model={settings.notesAnalysisModel}
      />
    </>
  );
}

/** "Test connection" card for the local LLM server: probes the configured
 * endpoint and reports reachable + model ids, or "Not running" with setup
 * guidance. Tests the endpoint as currently saved in settings (typed values
 * are saved on blur), so users see the same endpoint the analysis will use. */
function LlmConnectionCard({
  endpoint,
  model,
}: {
  endpoint: string;
  model: string;
}) {
  const [checking, setChecking] = useState(false);
  const [status, setStatus] = useState<LlmStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleTest = useCallback(async () => {
    setChecking(true);
    setError(null);
    setStatus(null);
    try {
      // Pass the configured endpoint explicitly so a value typed and saved on
      // blur is what's tested, even before any other settings round-trip.
      const next = await llmStatus(endpoint);
      setStatus(next);
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setChecking(false);
    }
  }, [endpoint]);

  const reachable = status?.reachable ?? false;

  return (
    <div className="llm-status-card">
      <div className="llm-status-head">
        <div className="llm-status-state">
          {status ? (
            <>
              <span
                className={`status-dot${reachable ? " success" : ""}`}
                aria-hidden="true"
              />
              <span className={`pill ${reachable ? "ready" : "error"}`}>
                {reachable ? "Connected" : "Not running"}
              </span>
            </>
          ) : (
            <span className="muted">Local AI server</span>
          )}
        </div>
        <button
          className="secondary-button"
          disabled={checking}
          onClick={() => void handleTest()}
          type="button"
        >
          {checking ? "Checking..." : "Test connection"}
        </button>
      </div>

      {status && reachable ? (
        status.models.length > 0 ? (
          <div className="llm-status-models">
            <p className="muted vocab-hint">
              {model.trim()
                ? `Available models (set to "${model.trim()}"):`
                : "Available models (an empty Model field uses the loaded one):"}
            </p>
            <div className="llm-model-pills">
              {status.models.map((id) => (
                <span
                  key={id}
                  className={`pill ${id === model.trim() ? "selected" : "preserve"}`}
                >
                  {id}
                </span>
              ))}
            </div>
          </div>
        ) : (
          <p className="muted vocab-hint">
            The server answered but has no model loaded. Load one, for example
            in LM Studio, so Scribe has something to run.
          </p>
        )
      ) : null}

      {status && !reachable ? (
        <p className="muted vocab-hint">
          No local AI server answered at{" "}
          <code>{status.endpoint || endpoint || "the configured endpoint"}</code>
          . Start LM Studio or Ollama, load a model, and make sure its local
          server is running at the address above (LM Studio uses{" "}
          <code>http://127.0.0.1:1234/v1</code>).
          {status.error ? ` ${status.error}` : ""}
        </p>
      ) : null}

      {error ? (
        <p className="muted vocab-hint" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** Single-line text setting that saves on blur (and on unmount, like the
 * vocabulary field) so every keystroke does not hit the settings command. */
function BlurSavedInput({
  ariaLabel,
  onSave,
  placeholder,
  value,
}: {
  ariaLabel: string;
  onSave: (value: string) => void;
  placeholder?: string;
  value: string;
}) {
  const [draft, setDraft] = useState(value);
  const latestRef = useRef({ draft, onSave, value });
  latestRef.current = { draft, onSave, value };

  useEffect(() => {
    setDraft(value);
  }, [value]);

  useEffect(
    () => () => {
      const latest = latestRef.current;
      if (latest.draft !== latest.value) {
        latest.onSave(latest.draft);
      }
    },
    [],
  );

  return (
    <input
      aria-label={ariaLabel}
      onBlur={() => {
        if (draft !== value) {
          onSave(draft);
        }
      }}
      onChange={(event) => setDraft(event.currentTarget.value)}
      placeholder={placeholder}
      type="text"
      value={draft}
    />
  );
}


function BlurSavedTextArea({
  ariaLabel,
  onSave,
  placeholder,
  rows = 3,
  value,
}: {
  ariaLabel: string;
  onSave: (value: string) => void;
  placeholder?: string;
  rows?: number;
  value: string;
}) {
  const [draft, setDraft] = useState(value);
  const latestRef = useRef({ draft, onSave, value });
  latestRef.current = { draft, onSave, value };

  useEffect(() => {
    setDraft(value);
  }, [value]);

  // Flush an unsaved draft if the view unmounts before blur fires.
  useEffect(
    () => () => {
      const latest = latestRef.current;
      if (latest.draft !== latest.value) {
        latest.onSave(latest.draft);
      }
    },
    [],
  );

  return (
    <textarea
      aria-label={ariaLabel}
      className="vocab-textarea"
      onBlur={() => {
        if (draft !== value) {
          onSave(draft);
        }
      }}
      onChange={(event) => setDraft(event.currentTarget.value)}
      placeholder={placeholder}
      rows={rows}
      value={draft}
    />
  );
}

/** Editable list of deterministic text replacements. Like the BlurSaved*
 * fields, edits live in a local draft and are flushed to settings on blur (and
 * on unmount), so typing never loses focus or hits the settings command per
 * keystroke. Add/remove persist immediately since there is nothing to debounce. */
function ReplacementsEditor({
  disabled,
  onChange,
  value,
}: {
  disabled?: boolean;
  onChange: (next: TextReplacement[]) => void;
  value: TextReplacement[];
}) {
  const [draft, setDraft] = useState<TextReplacement[]>(value);
  const latestRef = useRef({ draft, onChange, value });
  latestRef.current = { draft, onChange, value };

  // Adopt upstream changes (e.g. a settings reload) without clobbering an
  // in-progress edit: only resync when the saved value actually differs from
  // what we last pushed.
  useEffect(() => {
    setDraft(value);
  }, [value]);

  // Flush a pending row edit if the view unmounts before blur fires.
  useEffect(
    () => () => {
      const latest = latestRef.current;
      if (!replacementsEqual(latest.draft, latest.value)) {
        latest.onChange(latest.draft);
      }
    },
    [],
  );

  const flush = useCallback(() => {
    if (!replacementsEqual(latestRef.current.draft, latestRef.current.value)) {
      onChange(latestRef.current.draft);
    }
  }, [onChange]);

  const editRow = (index: number, patch: Partial<TextReplacement>) => {
    setDraft((rows) =>
      rows.map((row, i) => (i === index ? { ...row, ...patch } : row)),
    );
  };

  const addRow = () => {
    // Persist immediately so the new empty row is backed by saved state; an
    // empty `from` is ignored by the backend until the user fills it in.
    const next = [...latestRef.current.draft, { from: "", to: "" }];
    setDraft(next);
    onChange(next);
  };

  const removeRow = (index: number) => {
    const next = latestRef.current.draft.filter((_, i) => i !== index);
    setDraft(next);
    onChange(next);
  };

  return (
    <div className="replacements-editor">
      {draft.length === 0 ? (
        <p className="muted vocab-hint">No replacements yet.</p>
      ) : (
        <div className="replacement-rows">
          {draft.map((row, index) => (
            // Index key is intentional: rows are positional and reorder-free,
            // so this keeps each input mounted (and focused) across keystrokes.
            <div className="replacement-row" key={index}>
              <input
                aria-label={`When I say (row ${index + 1})`}
                disabled={disabled}
                onBlur={flush}
                onChange={(event) =>
                  editRow(index, { from: event.currentTarget.value })
                }
                placeholder="When I say…"
                type="text"
                value={row.from}
              />
              <span aria-hidden="true" className="replacement-arrow">
                →
              </span>
              <input
                aria-label={`Replace with (row ${index + 1})`}
                disabled={disabled}
                onBlur={flush}
                onChange={(event) =>
                  editRow(index, { to: event.currentTarget.value })
                }
                placeholder="Replace with…"
                type="text"
                value={row.to}
              />
              <button
                aria-label={`Remove replacement (row ${index + 1})`}
                className="replacement-remove"
                disabled={disabled}
                onClick={() => removeRow(index)}
                type="button"
              >
                <X aria-hidden="true" size={15} />
              </button>
            </div>
          ))}
        </div>
      )}
      <button
        className="secondary-button"
        disabled={disabled}
        onClick={addRow}
        type="button"
      >
        <Plus aria-hidden="true" size={15} />
        Add replacement
      </button>
    </div>
  );
}

/** Shallow value-equality for two replacement lists, so we only push to
 * settings when something actually changed. */
function replacementsEqual(a: TextReplacement[], b: TextReplacement[]): boolean {
  if (a.length !== b.length) {
    return false;
  }
  return a.every(
    (row, i) => row.from === b[i].from && row.to === b[i].to,
  );
}

// FILLER: parse the filler list into a clean, lowercased, de-duplicated array of
// single words. Split on commas, newlines, AND spaces, so "um, uh" and "um uh"
// both work. Matching is one transcribed word at a time, so multi-word entries
// ("you know") can't match; they're split into separate words here instead of
// silently stored as dead config.
function parseFillerWords(raw: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of raw.split(/[\s,]+/)) {
    const word = part.trim().toLowerCase();
    if (word && !seen.has(word)) {
      seen.add(word);
      out.push(word);
    }
  }
  return out;
}
