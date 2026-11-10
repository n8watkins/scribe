import { useCallback, useEffect, useRef, useState } from "react";
import {
  commandErrorMessage,
  LANGUAGE_AUTO,
  listModels,
  PARAKEET_V3_ID,
  PARAKEET_V3_LANGUAGES,
  llmStatus,
  SUPPORTED_LANGUAGES,
  type AppSettings,
  type LlmStatus,
  type ModelInfo,
} from "../backend";
import type { ViewActions } from "../types";
import { SettingRow } from "../components/layout";
import { MsInput, Toggle } from "../components/primitives";
import { currentPlatform } from "../lib/platform";
import { CustomWords } from "../components/CustomWords";
import { Replacements } from "../components/Replacements";

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

  // Parakeet detects the language itself and has no translate task.
  if (settings.selectedModelId === PARAKEET_V3_ID) {
    return (
      <SettingRow
        description="Parakeet detects which of its 25 languages you speak."
        more={`${PARAKEET_V3_LANGUAGES.join(", ")}. For other languages, or to translate into English, pick a Whisper model (99 languages).`}
        label="Language"
      >
        <span className="pill ready">Detected</span>
      </SettingRow>
    );
  }

  return (
    <>
      <SettingRow
        description="The language you speak."
          more="Auto-detect and anything but English need a multilingual model."
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
        description="Get English text from any language."
          more="Needs a multilingual model."
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
          Your model only understands English, so this won't take effect. Pick
          a multilingual one above, such as Small (Multilingual).
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
        description='Remove "um" and "uh" when you pause around them.'
        more='Fluent uses such as "oh no" or "like this" stay. Runs on this computer.'
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
        description="Words to remove, separated by commas."
          more="Riskier ones such as oh, like and so are safe to add because of the pause check."
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
            description="The pause around a filler before it goes."
          more="Lower removes more (100 to 1500 ms)."
            blockedBy={!settings.fillerSuppressionEnabled ? { reason: "Off because Remove filler words is off." } : null}
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
        description="A small bar on top of other windows while Scribe listens."
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
        blockedBy={!settings.showFloatingPill ? { reason: "Not shown while Show floating pill is off." } : null}
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
        description="Closing the window keeps Scribe in the tray."
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
        description="The Open Scribe hotkey also hides this window."
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
        description="A short message when a dictation finishes or fails."
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
        description="Adds a Developer page with diagnostics."
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

/** Names and words Whisper should expect (the context hint). */
export function ContextHintSettings({ actions, settings }: GroupProps) {
  return (
    <CustomWords actions={actions} settings={settings} TextArea={BlurSavedTextArea} />
  );
}

/** "Say this, write that" replacements, kept in replacements.txt. */
export function ReplacementsSettings({ actions, settings }: GroupProps) {
  return <Replacements actions={actions} settings={settings} />;
}

/** The one Local AI switch, the server connection, and the analysis prompt. */
export function LocalAiSettings({ actions, settings }: GroupProps) {
  return (
    <>
      <SettingRow
        description="Turns on note analysis and transform selection."
          more="Text goes only to the server below."
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
        description="LM Studio, Ollama or any OpenAI-compatible server."
          more="LM Studio uses http://127.0.0.1:1234/v1."
        blockedBy={!settings.notesAnalysisEnabled ? { reason: "Not used while Use local AI is off." } : null}
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
        blockedBy={!settings.notesAnalysisEnabled ? { reason: "Not used while Use local AI is off." } : null}
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
        description="Sent with each note you analyze. It decides what you get back."
        blockedBy={!settings.notesAnalysisEnabled ? { reason: "Not used while Use local AI is off." } : null}
                label="Analysis prompt"
        stacked
      >
        <BlurSavedTextArea
          ariaLabel="Notes analysis prompt"
          onSave={(notesAnalysisPrompt) =>
            actions.updateSettings({ notesAnalysisPrompt })
          }
          placeholder="Summarize this dictated note..."
          rows={3}
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
  disabled,
  onDraftChange,
  onSave,
  placeholder,
  rows = 3,
  value,
}: {
  ariaLabel: string;
  disabled?: boolean;
  /** Called on every keystroke, for live feedback such as a counter. */
  onDraftChange?: (value: string) => void;
  onSave: (value: string) => void;
  placeholder?: string;
  rows?: number;
  value: string;
}) {
  const [draft, setDraft] = useState(value);
  const latestRef = useRef({ draft, onSave, value });
  latestRef.current = { draft, onSave, value };
  const onDraftChangeRef = useRef(onDraftChange);
  onDraftChangeRef.current = onDraftChange;

  useEffect(() => {
    setDraft(value);
    onDraftChangeRef.current?.(value);
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
      disabled={disabled}
      onBlur={() => {
        if (draft !== value) {
          onSave(draft);
        }
      }}
      onChange={(event) => {
        setDraft(event.currentTarget.value);
        onDraftChange?.(event.currentTarget.value);
      }}
      placeholder={placeholder}
      rows={rows}
      value={draft}
    />
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
