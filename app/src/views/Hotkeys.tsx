import { useCallback, useEffect, useRef, useState } from "react";
import { Keyboard, RefreshCw, RotateCcw } from "lucide-react";
import {
  commandErrorMessage,
  getHotkeyStatus,
  rebindHotkey,
  resetHotkeysToDefaults,
  setHotkeyTrigger,
  type AppSettings,
  type HotkeyAction,
  type HotkeyBinding,
  type HotkeyStatus,
  type TriggerEdge,
} from "../backend";
import type { ViewActions } from "../types";
import { formatHotkey, hotkeyRows } from "../lib/format";
import { InlineError } from "../components/feedback";

const hotkeyActionLabels: Record<string, string> = {
  holdToTalk: "Hold-to-Talk",
  toggleDictation: "Toggle Dictation",
  pasteLastTranscript: "Paste Last Transcript",
  openDashboard: "Open Dashboard",
  discardDictation: "Discard / Cancel",
};

const hotkeyActionHints: Record<string, string> = {
  holdToTalk: "Hold to record, release to transcribe",
  toggleDictation: "Toggle dictation on and off",
  pasteLastTranscript: "Insert the Last Transcript Buffer",
  openDashboard: "Bring up this dashboard",
  discardDictation: "Cancel the current recording without transcribing",
};

const triggerLabels: Record<TriggerEdge, string> = {
  press: "On press",
  release: "On release",
};

/** The Toggle hint explains the note-chord trade-off right where the user
 * flips the edge: it is only available while Toggle acts on release. */
function toggleHint(trigger: TriggerEdge | null, noteChordActive: boolean): string {
  if (trigger === "press") {
    return "Acts on press — the hold-and-tap-Q note chord is off in this mode";
  }
  return noteChordActive
    ? "Acts on release — hold it and tap Q to dictate a note"
    : "Acts on release";
}

const hotkeyModifierOrder = ["Ctrl", "Shift", "Alt", "Super"] as const;

function modifierFromEventKey(key: string): string | null {
  switch (key) {
    case "Control":
      return "Ctrl";
    case "Shift":
      return "Shift";
    case "Alt":
      return "Alt";
    case "Meta":
      return "Super";
    default:
      return null;
  }
}

function captureKeyName(code: string): string | null {
  if (!code) {
    return null;
  }

  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) {
    return letter[1];
  }

  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) {
    return digit[1];
  }

  // Everything else already uses W3C code names the backend understands:
  // F1..F12, Space, Backquote, Minus, Comma, Enter, Tab, ArrowUp, ...
  return code;
}

function orderedModifiers(modifiers: Set<string>): string[] {
  return hotkeyModifierOrder.filter((modifier) => modifiers.has(modifier));
}

export function HotkeysView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [status, setStatus] = useState<HotkeyStatus | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [captureAction, setCaptureAction] = useState<HotkeyAction | null>(null);
  const [capturePreview, setCapturePreview] = useState("");
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [hotkeyBusy, setHotkeyBusy] = useState(false);
  const heldModifiersRef = useRef<Set<string>>(new Set());
  const captureCommittedRef = useRef(false);

  const loadStatus = useCallback(async () => {
    setStatusLoading(true);
    setStatusError(null);

    try {
      setStatus(await getHotkeyStatus());
    } catch (error) {
      setStatusError(commandErrorMessage(error));
    } finally {
      setStatusLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  const refreshSettings = actions.refresh;

  const applyRebind = useCallback(
    async (action: HotkeyAction, shortcut: string) => {
      setHotkeyBusy(true);
      setRowErrors((current) => {
        const next = { ...current };
        delete next[action];
        return next;
      });

      try {
        setStatus(await rebindHotkey(action, shortcut));
        await refreshSettings();
      } catch (error) {
        setRowErrors((current) => ({
          ...current,
          [action]: commandErrorMessage(error),
        }));
        try {
          setStatus(await getHotkeyStatus());
        } catch {
          // Keep the previous status if the refresh fails.
        }
      } finally {
        setHotkeyBusy(false);
      }
    },
    [refreshSettings],
  );

  const applyTrigger = useCallback(
    async (action: HotkeyAction, trigger: TriggerEdge) => {
      setHotkeyBusy(true);
      setRowErrors((current) => {
        const next = { ...current };
        delete next[action];
        return next;
      });

      try {
        setStatus(await setHotkeyTrigger(action, trigger));
        await refreshSettings();
      } catch (error) {
        setRowErrors((current) => ({
          ...current,
          [action]: commandErrorMessage(error),
        }));
        try {
          setStatus(await getHotkeyStatus());
        } catch {
          // Keep the previous status if the refresh fails.
        }
      } finally {
        setHotkeyBusy(false);
      }
    },
    [refreshSettings],
  );

  const startCapture = useCallback((action: HotkeyAction) => {
    heldModifiersRef.current = new Set();
    captureCommittedRef.current = false;
    setRowErrors((current) => {
      const next = { ...current };
      delete next[action];
      return next;
    });
    setCapturePreview("");
    setCaptureAction(action);
  }, []);

  const cancelCapture = useCallback(() => {
    captureCommittedRef.current = true;
    setCaptureAction(null);
    setCapturePreview("");
  }, []);

  useEffect(() => {
    if (!captureAction) {
      return;
    }

    const commit = (shortcut: string) => {
      if (captureCommittedRef.current) {
        return;
      }

      captureCommittedRef.current = true;
      setCaptureAction(null);
      setCapturePreview("");
      void applyRebind(captureAction, shortcut);
    };

    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();

      if (event.key === "Escape") {
        captureCommittedRef.current = true;
        setCaptureAction(null);
        setCapturePreview("");
        return;
      }

      const modifier = modifierFromEventKey(event.key);
      if (modifier) {
        heldModifiersRef.current.add(modifier);
        setCapturePreview(orderedModifiers(heldModifiersRef.current).join("+"));
        return;
      }

      const keyName = captureKeyName(event.code);
      if (!keyName) {
        return;
      }

      const modifiers = new Set<string>();
      if (event.ctrlKey) {
        modifiers.add("Ctrl");
      }
      if (event.shiftKey) {
        modifiers.add("Shift");
      }
      if (event.altKey) {
        modifiers.add("Alt");
      }
      if (event.metaKey) {
        modifiers.add("Super");
      }

      commit([...orderedModifiers(modifiers), keyName].join("+"));
    };

    const onKeyUp = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();

      const modifier = modifierFromEventKey(event.key);
      if (modifier && heldModifiersRef.current.size > 0) {
        // Modifier-only chord: released without a non-modifier key.
        commit(orderedModifiers(heldModifiersRef.current).join("+"));
      }
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);

    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [applyRebind, captureAction]);

  const handleResetDefaults = useCallback(async () => {
    setHotkeyBusy(true);
    setRowErrors({});
    setStatusError(null);

    try {
      setStatus(await resetHotkeysToDefaults());
      await refreshSettings();
    } catch (error) {
      setStatusError(commandErrorMessage(error));
    } finally {
      setHotkeyBusy(false);
    }
  }, [refreshSettings]);

  const triggerFromSettings = (action: string): TriggerEdge | null => {
    switch (action) {
      case "toggleDictation":
        return settings.hotkeys.toggleDictationTrigger;
      case "pasteLastTranscript":
        return settings.hotkeys.pasteLastTranscriptTrigger;
      case "openDashboard":
        return settings.hotkeys.openDashboardTrigger;
      case "discardDictation":
        return settings.hotkeys.discardDictationTrigger;
      default:
        return null;
    }
  };

  const bindings: HotkeyBinding[] =
    status?.bindings ??
    hotkeyRows(settings).map((row) => ({
      action: row.action,
      shortcut: row.value,
      normalizedShortcut: null,
      trigger: triggerFromSettings(row.action),
      registered: false,
      error: null,
    }));

  const noteChordActive = status?.noteChordActive ?? false;

  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Global hotkeys</h2>
          <div className="row-actions">
            <button
              className="ghost-button"
              disabled={statusLoading}
              onClick={() => void loadStatus()}
              type="button"
            >
              <RefreshCw aria-hidden="true" size={13} />
              Refresh
            </button>
            <button
              className="secondary-button"
              disabled={hotkeyBusy || captureAction !== null}
              onClick={() => void handleResetDefaults()}
              type="button"
            >
              <RotateCcw aria-hidden="true" size={13} />
              Reset to defaults
            </button>
          </div>
        </div>
        {statusError ? (
          <InlineError message={statusError} onRetry={loadStatus} />
        ) : null}
        {statusLoading && !status ? (
          <div className="pending-panel">
            <RefreshCw aria-hidden="true" size={14} />
            <span>Loading hotkey registration status...</span>
          </div>
        ) : (
          <div className="hotkey-editor-list">
            {bindings.map((binding) => {
              const isCapturing = captureAction === binding.action;
              const rowError = rowErrors[binding.action] ?? binding.error;
              return (
                <div className="hotkey-editor-row" key={binding.action}>
                  <div>
                    <strong>
                      {hotkeyActionLabels[binding.action] ?? binding.action}
                    </strong>
                    {rowError ? (
                      <span className="hotkey-error">{rowError}</span>
                    ) : (
                      <span>
                        {binding.action === "toggleDictation"
                          ? toggleHint(binding.trigger, noteChordActive)
                          : (hotkeyActionHints[binding.action] ??
                            "Global shortcut")}
                      </span>
                    )}
                    {binding.trigger ? (
                      <div
                        className="trigger-segments segmented-control"
                        role="group"
                        aria-label="Trigger edge"
                      >
                        {(["press", "release"] as const).map((edge) => (
                          <button
                            key={edge}
                            type="button"
                            className={
                              binding.trigger === edge ? "active-segment" : ""
                            }
                            disabled={hotkeyBusy || captureAction !== null}
                            onClick={() =>
                              void applyTrigger(
                                binding.action as HotkeyAction,
                                edge,
                              )
                            }
                          >
                            {triggerLabels[edge]}
                          </button>
                        ))}
                      </div>
                    ) : null}
                  </div>
                  <kbd>
                    {isCapturing
                      ? capturePreview
                        ? `${formatHotkey(capturePreview)} ...`
                        : "Press keys..."
                      : formatHotkey(binding.shortcut)}
                  </kbd>
                  {status ? (
                    binding.registered ? (
                      <span className="pill ready">Registered</span>
                    ) : (
                      <span className="pill error">
                        {binding.error ? "Failed" : "Inactive"}
                      </span>
                    )
                  ) : (
                    <span className="pill idle">Unknown</span>
                  )}
                  {isCapturing ? (
                    <button
                      className="secondary-button"
                      onClick={cancelCapture}
                      type="button"
                    >
                      Press keys... (Esc to cancel)
                    </button>
                  ) : (
                    <button
                      className="secondary-button"
                      disabled={hotkeyBusy || captureAction !== null}
                      onClick={() => startCapture(binding.action as HotkeyAction)}
                      type="button"
                    >
                      <Keyboard aria-hidden="true" size={13} />
                      Rebind
                    </button>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {status?.windowsFallbackNote ? (
          <p className="muted hotkey-note">{status.windowsFallbackNote}</p>
        ) : null}
        {status?.holdReleaseVerificationRequired ? (
          <p className="muted hotkey-note">
            Hold-to-talk release is tracked by a native key watcher, so
            modifier-only chords like Ctrl+Shift work for holding.
          </p>
        ) : null}
      </article>
    </section>
  );
}
