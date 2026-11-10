import { useState } from "react";
import { Scissors, Type } from "lucide-react";
import type { AppSettings } from "../backend";
import type { ViewActions } from "../types";
import {
  HINT_CHAR_LIMIT,
  hintAsSentence,
  trimHintToChars,
} from "../lib/promptBudget";
import { isParakeetModel } from "../lib/engine";

const PLACEHOLDER =
  "My notes mention Scribe, Tauri, whisper.cpp and Claude Code.";

/** Words page > Custom words: the hint with a character counter, the "write
 * it as a sentence" helper, and a clear note when the engine in use
 * (Parakeet) doesn't read it. */
export function CustomWords({
  actions,
  settings,
  TextArea,
}: {
  actions: ViewActions;
  settings: AppSettings;
  TextArea: (props: {
    ariaLabel: string;
    disabled?: boolean;
    onDraftChange?: (value: string) => void;
    onSave: (value: string) => void;
    placeholder?: string;
    rows?: number;
    value: string;
  }) => React.ReactNode;
}) {
  const [draft, setDraft] = useState(settings.vocabularyPrompt);
  const parakeet = isParakeetModel(settings.selectedModelId);
  const length = draft.trim().length;
  const over = length > HINT_CHAR_LIMIT;
  const save = (vocabularyPrompt: string) =>
    actions.updateSettings({ vocabularyPrompt });

  return (
    <div className={parakeet ? "custom-words is-unused" : "custom-words"}>
      {parakeet ? (
        <p className="custom-words-unused" role="status">
          <strong>Not used by Parakeet.</strong> Kept for Whisper models.
          Replacements work with every model.
        </p>
      ) : (
        <p className="muted vocab-hint">
          Names and jargon Whisper should expect. Write them as a sentence,
          most important first.
        </p>
      )}
      <div className="custom-words-box">
        <TextArea
          ariaLabel="Custom words"
          disabled={parakeet}
          onDraftChange={setDraft}
          onSave={save}
          placeholder={PLACEHOLDER}
          rows={4}
          value={settings.vocabularyPrompt}
        />
        <span
          aria-live="polite"
          className={over ? "custom-words-count is-over" : "custom-words-count"}
          title={`Whisper reads about ${HINT_CHAR_LIMIT} characters of custom words`}
        >
          {length} / {HINT_CHAR_LIMIT}
        </span>
      </div>
      {!parakeet && over ? (
        <p className="hint-warning" role="alert">
          Over by {length - HINT_CHAR_LIMIT} characters. Whisper won't see the
          end. Move words it keeps getting wrong to Replacements.
        </p>
      ) : null}
      {!parakeet && draft.trim() ? (
        <div className="button-row">
          {draft.includes(",") && !/[.!?]\s*$/.test(draft.trim()) ? (
            <button
              className="ghost-button"
              disabled={actions.savingSettings}
              onClick={() => save(hintAsSentence(draft))}
              title="Whisper follows a sentence better than a list"
              type="button"
            >
              <Type aria-hidden="true" size={14} />
              Write it as a sentence
            </button>
          ) : null}
          {over ? (
            <button
              className="ghost-button"
              disabled={actions.savingSettings}
              onClick={() => save(trimHintToChars(draft))}
              type="button"
            >
              <Scissors aria-hidden="true" size={14} />
              Keep what fits
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
