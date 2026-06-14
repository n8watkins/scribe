import { useState } from "react";
import {
  Clipboard,
  Database,
  Download,
  Gauge,
  Keyboard,
  Mic,
  Wand2,
} from "lucide-react";
import {
  commandErrorMessage,
  transformSelection,
  type DashboardData,
  type MicrophoneInfo,
  type ModelInfo,
} from "../backend";
import type { ToastNotice, ViewActions, ViewName } from "../types";
import {
  formatHotkey,
  isAutoInsert,
  isSelectedModelReady,
  microphoneDisplayName,
  statusCardValue,
} from "../lib/format";
import { HotkeyList, StatusCard } from "../components/layout";
import { LastTranscriptCard } from "../components/transcript";

export function DashboardView({
  actions,
  data,
  microphones,
  models,
  setActiveView,
  showNotice,
}: {
  actions: ViewActions;
  data: DashboardData;
  microphones: MicrophoneInfo[] | null;
  models: ModelInfo[] | null;
  setActiveView: (view: ViewName) => void;
  showNotice?: (message: string, tone?: ToastNotice["tone"]) => void;
}) {
  const { appState, lastTranscript, settings } = data;
  const modelReady = isSelectedModelReady(models, settings.selectedModelId);

  return (
    <>
      {!modelReady ? (
        <div className="setup-banner" role="alert">
          <Download aria-hidden="true" size={16} />
          <div>
            <strong>Download a model to start dictating</strong>
            <span>
              Transcription needs a local Whisper model on this device.
            </span>
          </div>
          <button
            className="primary-button"
            onClick={() => setActiveView("Models")}
            type="button"
          >
            Get a model
          </button>
        </div>
      ) : null}

      <section className="status-grid" aria-label="Current setup">
        <StatusCard
          action="Open Transcribe"
          Icon={Gauge}
          label="Status"
          onAction={() => setActiveView("Transcribe")}
          value={statusCardValue(appState.status)}
        />
        <StatusCard
          action="Open Audio"
          Icon={Mic}
          label="Active mic"
          onAction={() => setActiveView("Audio")}
          status={<span className="status-dot success" />}
          value={microphoneDisplayName(microphones, settings.selectedMicId)}
        />
        <StatusCard
          action="Open Models"
          Icon={Database}
          label="Active model"
          onAction={() => setActiveView("Models")}
          value={settings.selectedModelId ?? "Choose a model"}
        />
        <StatusCard
          action="Open Settings"
          Icon={Clipboard}
          label="Auto-insert"
          onAction={() => setActiveView("Settings")}
          value={isAutoInsert(settings.outputMode) ? "On" : "Off"}
        />
      </section>

      <section className="main-grid">
        <LastTranscriptCard
          clearing={actions.clearingLastTranscript}
          copying={actions.copyingLastTranscript}
          onClear={actions.clearLastTranscript}
          onCopy={actions.copyLastTranscript}
          onPaste={actions.pasteLastTranscript}
          pasting={actions.pastingLastTranscript}
          settings={settings}
          transcript={lastTranscript}
        />

        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Hotkeys</h2>
            <button
              className="ghost-button"
              onClick={() => setActiveView("Hotkeys")}
              type="button"
            >
              <Keyboard aria-hidden="true" size={15} />
              Rebind
            </button>
          </div>
          <HotkeyList compact settings={settings} />
        </article>

        <TransformSelectionPanel
          shortcut={settings.hotkeys.transformSelection}
          showNotice={showNotice}
        />
      </section>
    </>
  );
}

/** Inline AI editor: the user highlights text in any app, types an instruction
 * here, and Scribe copies the selection, rewrites it with the local LLM, and
 * pastes the result back over the selection. This is the typed-instruction v1
 * of the selected-text transform; the matching hotkey opens this panel. */
function TransformSelectionPanel({
  shortcut,
  showNotice,
}: {
  shortcut: string;
  showNotice?: (message: string, tone?: ToastNotice["tone"]) => void;
}) {
  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    const trimmed = instruction.trim();
    if (!trimmed || busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // The focused app keeps the highlighted selection; the backend sends
      // Ctrl+C to grab it, transforms it, and pastes the result back.
      const result = await transformSelection(trimmed);
      showNotice?.(result.message ?? "Selection transformed.", "success");
    } catch (caught) {
      const message = commandErrorMessage(caught);
      setError(message);
      showNotice?.(message, "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <article className="panel-card">
      <div className="section-heading compact">
        <h2>Transform selection</h2>
        <span className="muted">{formatHotkey(shortcut)}</span>
      </div>
      <p className="muted">
        Highlight text in any app, describe the change, and Scribe rewrites it in
        place with your local LLM.
      </p>
      <textarea
        className="transform-instruction"
        aria-label="Transform instruction"
        placeholder="e.g. make this concise, translate to Spanish, fix grammar"
        rows={2}
        value={instruction}
        onChange={(event) => setInstruction(event.target.value)}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault();
            void run();
          }
        }}
        disabled={busy}
      />
      <div className="section-heading compact">
        <button
          className="primary-button"
          onClick={() => void run()}
          type="button"
          disabled={busy || instruction.trim().length === 0}
        >
          <Wand2 aria-hidden="true" size={15} />
          {busy ? "Transforming..." : "Transform selection"}
        </button>
      </div>
      {error ? (
        <p className="muted" role="alert">
          {error}
        </p>
      ) : null}
    </article>
  );
}
