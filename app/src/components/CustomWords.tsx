import { useState } from "react";
import { AlertTriangle, Scissors, Type } from "lucide-react";
import type { AppSettings } from "../backend";
import type { ViewActions } from "../types";
import {
  analyzeHint,
  hintAsSentence,
  HINT_TOKEN_BUDGET,
  PROMPT_TOKEN_LIMIT,
  trimHintToBudget,
} from "../lib/promptBudget";
import { isParakeetModel } from "../lib/engine";

const PLACEHOLDER =
  "My notes mention Scribe, Tauri, whisper.cpp and Claude Code.";

/** Words page > Custom words: the hint text with a live budget meter, the
 * "write it as a sentence" helper, and a clear note when the engine in use
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
  const budget = analyzeHint(draft);
  const save = (vocabularyPrompt: string) =>
    actions.updateSettings({ vocabularyPrompt });
  const percent = Math.min(100, (budget.tokens / HINT_TOKEN_BUDGET) * 100);
  const tone = budget.overLimit ? "over" : budget.overBudget ? "near" : "ok";

  return (
    <div className={parakeet ? "custom-words is-unused" : "custom-words"}>
      {parakeet ? (
        <p className="custom-words-unused" role="status">
          <strong>Not used by Parakeet.</strong> Parakeet can't take a word
          list. Your words are kept for Whisper; Replacements work with every
          model.
        </p>
      ) : (
        <p className="muted vocab-hint">
          Names and jargon Whisper should expect. Write them as a sentence,
          most important first.
        </p>
      )}
      <TextArea
        ariaLabel="Custom words"
        disabled={parakeet}
        onDraftChange={setDraft}
        onSave={save}
        placeholder={PLACEHOLDER}
        rows={4}
        value={settings.vocabularyPrompt}
      />
      {!parakeet && draft.trim() ? (
        <>
          <div className={`hint-meter is-${tone}`}>
            <div className="hint-meter-bar" aria-hidden="true">
              <span style={{ width: `${percent}%` }} />
            </div>
            <span className="hint-meter-text">
              About {budget.tokens} of {HINT_TOKEN_BUDGET} tokens,{" "}
              {budget.terms} {budget.terms === 1 ? "term" : "terms"}. Room for
              about 35 to 40.
            </span>
          </div>
          {budget.overLimit ? (
            <p className="hint-warning" role="alert">
              <AlertTriangle aria-hidden="true" size={14} />
              <span>
                Whisper reads {PROMPT_TOKEN_LIMIT} tokens at most, so the last{" "}
                {budget.dropped.length} terms are never used:{" "}
                {budget.dropped.slice(0, 6).join(", ")}
                {budget.dropped.length > 6 ? " and more" : ""}. Move the ones
                it keeps getting wrong to Replacements.
              </span>
            </p>
          ) : budget.overBudget ? (
            <p className="hint-warning is-soft" role="status">
              <AlertTriangle aria-hidden="true" size={14} />
              <span>
                Nearly full. Live transcription gets less of what you just
                said to go on.
              </span>
            </p>
          ) : null}
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
            {budget.overBudget ? (
              <button
                className="ghost-button"
                disabled={actions.savingSettings}
                onClick={() => save(trimHintToBudget(draft))}
                type="button"
              >
                <Scissors aria-hidden="true" size={14} />
                Keep what fits
              </button>
            ) : null}
          </div>
        </>
      ) : null}
    </div>
  );
}
