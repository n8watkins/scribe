import {
  Clipboard,
  Database,
  Download,
  Gauge,
  Keyboard,
  Mic,
} from "lucide-react";
import type { DashboardData, MicrophoneInfo, ModelInfo } from "../backend";
import type { ViewActions, ViewName } from "../types";
import {
  clipboardStatus,
  isSelectedModelReady,
  microphoneDisplayName,
  outputModeLabel,
  statusCardValue,
} from "../lib/format";
import { HotkeyList, StatusCard } from "../components/layout";
import { StatePill } from "../components/primitives";
import { LastTranscriptCard } from "../components/transcript";

export function DashboardView({
  actions,
  data,
  microphones,
  models,
  setActiveView,
}: {
  actions: ViewActions;
  data: DashboardData;
  microphones: MicrophoneInfo[] | null;
  models: ModelInfo[] | null;
  setActiveView: (view: ViewName) => void;
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
          action="Record"
          Icon={Gauge}
          label="Current status"
          onAction={() => setActiveView("Transcribe")}
          status={<StatePill appState={appState} />}
          value={statusCardValue(appState.status)}
        />
        <StatusCard
          action="Choose"
          Icon={Mic}
          label="Active microphone"
          onAction={() => setActiveView("Audio")}
          status={<span className="status-dot success" />}
          value={microphoneDisplayName(microphones, settings.selectedMicId)}
        />
        <StatusCard
          action="Manage"
          Icon={Database}
          label="Active model"
          onAction={() => setActiveView("Models")}
          value={settings.selectedModelId ?? "Choose a model"}
        />
        <StatusCard
          action="Change"
          Icon={Clipboard}
          label="Output mode"
          onAction={() => setActiveView("Settings")}
          status={<span className="pill preserve">{clipboardStatus(settings)}</span>}
          value={outputModeLabel(settings.outputMode)}
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
      </section>
    </>
  );
}
