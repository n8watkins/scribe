import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  BookOpen,
  Plus,
  SlidersHorizontal,
  Sparkles,
  X,
} from "lucide-react";
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
import {
  formatHotkey,
  isAutoInsert,
  isKeepClipboard,
  outputModeFromToggles,
} from "../lib/format";
import { SettingRow } from "../components/layout";
import { MsInput, Toggle } from "../components/primitives";

export function SettingsView({
  actions,
  initialTabId,
  settings,
}: {
  actions: ViewActions;
  initialTabId?: string | null;
  settings: AppSettings;
}) {
  // The selected model's multilingual capability drives the pair-guard notice
  // shown when a non-English language (or translate) is chosen against an
  // English-only model. Loaded from the catalog; null while it loads (the
  // notice stays hidden until we actually know the model is English-only, so it
  // never flashes for the default English/English-model setup).
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
          (model) =>
            model.selected || model.id === settings.selectedModelId,
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

  // The user asked for non-English work (a specific non-English language, or
  // auto-detect, or translate-to-English) but the active model is English-only.
  const wantsMultilingual =
    settings.translateToEnglish ||
    settings.language === LANGUAGE_AUTO ||
    (settings.language !== "en" && settings.language.length > 0);
  const showMultilingualNotice =
    wantsMultilingual && selectedModelMultilingual === false;

  const tabs: {
    id: string;
    title: string;
    icon: ReactNode;
    render: () => ReactNode;
  }[] = [
    {
      id: "output",
      title: "App & output",
      icon: <SlidersHorizontal aria-hidden="true" size={16} />,
      render: () => (
      <article className="panel-card">
        <div className="settings-list">
          <div className="settings-subsection">
            <h3 className="settings-subhead">Output</h3>
            <SettingRow
              description={`On: your transcript is pasted automatically when you stop talking. Off: it's saved to the buffer — paste it anywhere with ${formatHotkey(settings.hotkeys.pasteLastTranscript)}.`}
              label="Auto-insert after dictation"
            >
              <Toggle
                checked={isAutoInsert(settings.outputMode)}
                disabled={actions.savingSettings}
                label="Auto-insert after dictation"
                onChange={(on) =>
                  actions.updateSettings({
                    outputMode: outputModeFromToggles(
                      on,
                      isKeepClipboard(settings.outputMode),
                    ),
                  })
                }
              />
            </SettingRow>
            <SettingRow
              description="On: Scribe borrows your clipboard for the paste, then restores what you had — Ctrl+V keeps working with your stuff. Off: it leaves the transcript on your clipboard so you paste it yourself with any bind (Ctrl+V / Shift+Insert), replacing your previous clipboard."
              label="Keep my clipboard"
            >
              <Toggle
                checked={isKeepClipboard(settings.outputMode)}
                disabled={actions.savingSettings}
                label="Keep my clipboard"
                onChange={(on) =>
                  actions.updateSettings({
                    outputMode: outputModeFromToggles(
                      isAutoInsert(settings.outputMode),
                      on,
                    ),
                  })
                }
              />
            </SettingRow>
            <SettingRow
              description="Spoken language Scribe transcribes. Auto-detect and any non-English language need a multilingual model (see Models)."
              label="Language"
            >
              <select
                disabled={actions.savingSettings}
                onChange={(event) =>
                  actions.updateSettings({
                    language: event.currentTarget.value,
                  })
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
              description="Transcribe any spoken language and output English instead (Whisper's translate task). Needs a multilingual model."
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
                Your selected model is English-only, so this setting won't take
                effect. Download and select a multilingual model (e.g. Base or
                Small Multilingual) from the Models tab.
              </p>
            ) : null}
          </div>

          {/* FILLER: pause-aware filler suppression */}
          <div className="settings-subsection">
            <h3 className="settings-subhead">Filler suppression</h3>
            <SettingRow
              description={
                'Remove filler words (um, uh, …) — but only when there’s a real pause around them, so fluent uses ("oh no", "like this") are kept. Deterministic; runs on the device, no LLM.'
              }
              label="Remove pause-bracketed filler words"
            >
              <Toggle
                checked={settings.fillerSuppressionEnabled}
                disabled={actions.savingSettings}
                label="Remove pause-bracketed filler words"
                onChange={(fillerSuppressionEnabled) =>
                  actions.updateSettings({ fillerSuppressionEnabled })
                }
              />
            </SettingRow>
            <SettingRow
              description="The words to remove, comma-separated. Add the risky ones (oh, like, so) only if you want — the pause check guards them. Clearing this removes nothing."
              label="Filler words"
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
            <SettingRow
              description="How long an adjacent silence must be (ms) for a filler to count as a hesitation worth removing. Lower = more aggressive (100–1500 ms)."
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
            <p className="muted vocab-hint">
              Uses Whisper's own word timings; no LLM, and it runs on the warm
              transcription server, so there's no real speed cost.
            </p>
          </div>

          <div className="settings-subsection">
            <h3 className="settings-subhead">Floating pill</h3>
            <SettingRow
              description="Always-on-top capture status overlay."
              label="Show floating status pill"
            >
              <Toggle
                checked={settings.showFloatingPill}
                disabled={actions.savingSettings}
                label="Show floating status pill"
                onChange={(showFloatingPill) =>
                  actions.updateSettings({ showFloatingPill })
                }
              />
            </SettingRow>
            <SettingRow
              description="What the pill shows while you dictate."
              label="Pill display mode"
            >
              <select
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
                <option value="visualizer">Visualizer</option>
                <option value="visualizer_with_text">Visualizer + text</option>
              </select>
            </SettingRow>
            <SettingRow
              description="Waveform and dot color for normal dictation on the pill."
              label="Pill color"
            >
              <input
                aria-label="Pill color"
                disabled={actions.savingSettings}
                onChange={(event) =>
                  actions.updateSettings({
                    pillColorNormal: event.currentTarget.value,
                  })
                }
                type="color"
                value={settings.pillColorNormal}
              />
            </SettingRow>
            <SettingRow
              description="Waveform and dot color while taking a note (tilde+Q)."
              label="Note pill color"
            >
              <input
                aria-label="Note pill color"
                disabled={actions.savingSettings}
                onChange={(event) =>
                  actions.updateSettings({
                    pillColorNote: event.currentTarget.value,
                  })
                }
                type="color"
                value={settings.pillColorNote}
              />
            </SettingRow>
            <SettingRow
              description="Background color of the floating pill."
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
          </div>

          <div className="settings-subsection">
            <h3 className="settings-subhead">App</h3>
            <SettingRow
              description="Start Scribe when Windows starts."
              label="Launch at startup"
            >
              <Toggle
                checked={settings.launchAtStartup}
                disabled={actions.savingSettings}
                label="Launch at startup"
                onChange={(launchAtStartup) =>
                  actions.updateSettings({ launchAtStartup })
                }
              />
            </SettingRow>
            <SettingRow
              description="Keep the app available from the system tray."
              label="Minimize to tray"
            >
              <Toggle
                checked={settings.minimizeToTray}
                disabled={actions.savingSettings}
                label="Minimize to tray"
                onChange={(minimizeToTray) =>
                  actions.updateSettings({ minimizeToTray })
                }
              />
            </SettingRow>
            <SettingRow
              description="The dashboard hotkey minimizes the window again when it is already focused."
              label="Dashboard hotkey toggles"
            >
              <Toggle
                checked={settings.dashboardHotkeyToggles}
                disabled={actions.savingSettings}
                label="Dashboard hotkey toggles"
                onChange={(dashboardHotkeyToggles) =>
                  actions.updateSettings({ dashboardHotkeyToggles })
                }
              />
            </SettingRow>
            <SettingRow
              description="Display completion and failure notices."
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
            <SettingRow description="Play start and stop capture tones." label="Sounds">
              <Toggle
                checked={settings.soundsEnabled}
                disabled={actions.savingSettings}
                label="Sounds"
                onChange={(soundsEnabled) => actions.updateSettings({ soundsEnabled })}
              />
            </SettingRow>
            <SettingRow
              description="Show a Developer panel in the sidebar with diagnostics like the live window resolution."
              label="Enable developer settings"
            >
              <Toggle
                checked={settings.developerSettingsEnabled}
                disabled={actions.savingSettings}
                label="Enable developer settings"
                onChange={(developerSettingsEnabled) =>
                  actions.updateSettings({ developerSettingsEnabled })
                }
              />
            </SettingRow>
          </div>
        </div>
      </article>
      ),
    },
    {
      id: "dictionary",
      title: "Dictionary",
      icon: <BookOpen aria-hidden="true" size={16} />,
      render: () => (
      <article className="panel-card">
        <div className="settings-list">
        <div className="dictionary-subsection">
          <h3 className="dictionary-subhead">Context hint</h3>
          <p className="muted vocab-hint">
            A short, natural-language note that <em>primes</em> Whisper toward
            your names, jargon, and spellings, e.g. "Tauri, natkins,
            whisper.cpp". It nudges recognition toward these words — it is not
            find-and-replace, so it can't guarantee an exact output. For
            guaranteed swaps, use Replacements below.
          </p>
          <BlurSavedTextArea
            ariaLabel="Context hint"
            onSave={(vocabularyPrompt) =>
              actions.updateSettings({ vocabularyPrompt })
            }
            placeholder="Tauri, natkins, whisper.cpp"
            value={settings.vocabularyPrompt}
          />
        </div>
        <div className="dictionary-subsection">
          <h3 className="dictionary-subhead">Replacements</h3>
          <p className="muted vocab-hint">
            Deterministic: whenever you say the phrase on the left, Scribe writes
            the text on the right — applied after transcription. E.g. "my email"
            → your address, or fix "clawed" → "Claude". Matching is
            case-insensitive and on whole words.
          </p>
          <ReplacementsEditor
            disabled={actions.savingSettings}
            onChange={(textReplacements) =>
              actions.updateSettings({ textReplacements })
            }
            value={settings.textReplacements}
          />
        </div>
        </div>
      </article>
      ),
    },
    {
      id: "notes",
      title: "Notes",
      icon: <Sparkles aria-hidden="true" size={16} />,
      render: () => (
      <article className="panel-card">
        <div className="settings-list">
        <SettingRow
          description="Analyze notes on demand from the Notes view using a local LLM server (LM Studio, Ollama, or any OpenAI-compatible API). Nothing leaves this machine."
          label="Analyze notes with a local LLM"
        >
          <Toggle
            checked={settings.notesAnalysisEnabled}
            disabled={actions.savingSettings}
            label="Analyze notes with a local LLM"
            onChange={(notesAnalysisEnabled) =>
              actions.updateSettings({ notesAnalysisEnabled })
            }
          />
        </SettingRow>
        <SettingRow
          description="Base URL of the local server's OpenAI-compatible API. LM Studio's default is http://127.0.0.1:1234/v1."
          label="Server endpoint"
        >
          <BlurSavedInput
            ariaLabel="Notes analysis server endpoint"
            onSave={(notesAnalysisEndpoint) =>
              actions.updateSettings({ notesAnalysisEndpoint })
            }
            placeholder="http://127.0.0.1:1234/v1"
            value={settings.notesAnalysisEndpoint}
          />
        </SettingRow>
        <SettingRow
          description="Leave empty to use whatever model the server has loaded."
          label="Model"
        >
          <BlurSavedInput
            ariaLabel="Notes analysis model"
            onSave={(notesAnalysisModel) =>
              actions.updateSettings({ notesAnalysisModel })
            }
            placeholder="Loaded model (automatic)"
            value={settings.notesAnalysisModel}
          />
        </SettingRow>
        <p className="muted vocab-hint">
          Analysis prompt — the note text is sent along with this instruction,
          so it alone decides what analysis produces (summary, action items,
          tags, ...).
        </p>
        <BlurSavedTextArea
          ariaLabel="Notes analysis prompt"
          onSave={(notesAnalysisPrompt) =>
            actions.updateSettings({ notesAnalysisPrompt })
          }
          placeholder="Summarize this dictated note..."
          rows={4}
          value={settings.notesAnalysisPrompt}
        />
        <LlmConnectionCard
          endpoint={settings.notesAnalysisEndpoint}
          model={settings.notesAnalysisModel}
        />
        </div>
      </article>
      ),
    },
  ];

  const [activeTab, setActiveTab] = useState(
    initialTabId && tabs.some((tab) => tab.id === initialTabId)
      ? initialTabId
      : tabs[0].id,
  );
  const active = tabs.find((tab) => tab.id === activeTab) ?? tabs[0];

  // Deep-links from other views (e.g. the Notes/History "Settings" buttons)
  // pass a target tab id; select it whenever it changes to a known tab.
  useEffect(() => {
    if (initialTabId && tabs.some((tab) => tab.id === initialTabId)) {
      setActiveTab(initialTabId);
    }
    // `tabs` is rebuilt every render; key the effect on the id alone.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialTabId]);

  return (
    <section className="view-grid">
      <div className="settings-tabs" role="tablist" aria-label="Settings sections">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            id={`settings-tab-${tab.id}`}
            aria-controls={`settings-panel-${tab.id}`}
            aria-selected={tab.id === active.id}
            className={`settings-tab${tab.id === active.id ? " is-active" : ""}`}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.icon}
            <span>{tab.title}</span>
          </button>
        ))}
      </div>
      <div
        className="settings-tabpanel"
        role="tabpanel"
        id={`settings-panel-${active.id}`}
        aria-labelledby={`settings-tab-${active.id}`}
      >
        {active.render()}
      </div>
    </section>
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
            <span className="muted">Local LLM server connection</span>
          )}
        </div>
        <button
          className="secondary-button"
          disabled={checking}
          onClick={() => void handleTest()}
          type="button"
        >
          {checking ? "Checking…" : "Test connection"}
        </button>
      </div>

      {status && reachable ? (
        status.models.length > 0 ? (
          <div className="llm-status-models">
            <p className="muted vocab-hint">
              {model.trim()
                ? `Available models (set to "${model.trim()}"):`
                : "Available models (empty Model field uses the first/loaded one):"}
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
            The server is reachable but has no models loaded. Load a model (e.g.
            in LM Studio) so analysis has something to run.
          </p>
        )
      ) : null}

      {status && !reachable ? (
        <p className="muted vocab-hint">
          No local LLM server answered at{" "}
          <code>{status.endpoint || endpoint || "the configured endpoint"}</code>
          . Start LM Studio (or Ollama), load a model, and make sure its local
          server is running at the endpoint above (LM Studio's default is{" "}
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
// ("you know") can't match — they're split into separate words here instead of
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
