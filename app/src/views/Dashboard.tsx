import { useEffect, useState } from "react";
import {
  Clipboard,
  Database,
  Download,
  Gauge,
  Keyboard,
  Mic,
} from "lucide-react";
import {
  getHotkeyStatus,
  type DashboardData,
  type MicrophoneInfo,
  type ModelInfo,
  type PartialTranscriptEvent,
} from "../backend";
import type { ViewActions, ViewName } from "../types";
import {
  afterDictationOf,
  isSelectedModelReady,
  microphoneDisplayName,
  statusCardValue,
} from "../lib/format";
import { HotkeyList, StatsRow, StatusCard } from "../components/layout";
import { LastTranscriptCard, LiveTranscript } from "../components/transcript";
import { WhatsNewCard } from "../components/ReleaseNotes";

/** The hotkeys Home always lists; any other hotkey shows up only when it
 * failed to register, so a conflict is never hidden. */
const HOME_HOTKEYS = ["holdToTalk", "toggleDictation", "pasteLastTranscript"];

const AFTER_DICTATION_LABELS = {
  insert: "Insert at cursor",
  copy: "Copy only",
  save: "Save only",
} as const;

export function DashboardView({
  actions,
  data,
  liveTranscript,
  microphones,
  models,
  setActiveView,
}: {
  actions: ViewActions;
  data: DashboardData;
  liveTranscript: PartialTranscriptEvent | null;
  microphones: MicrophoneInfo[] | null;
  models: ModelInfo[] | null;
  setActiveView: (view: ViewName) => void;
}) {
  const { appState, lastTranscript, settings, stats } = data;
  const modelReady = isSelectedModelReady(models, settings.selectedModelId);
  const failedHotkeys = useFailedHotkeys(
    JSON.stringify(settings.hotkeys),
    settings.notesAnalysisEnabled,
  );
  const shownHotkeys = new Set([...HOME_HOTKEYS, ...failedHotkeys]);
  const modelName =
    models?.find((model) => model.id === settings.selectedModelId)?.name ??
    settings.selectedModelId ??
    "Choose a model";
  const liveText = liveTranscript?.text.trim() ?? "";
  const showLive =
    liveText.length > 0 &&
    (appState.status === "Recording" ||
      appState.status === "Stopping" ||
      appState.status === "Transcribing");

  return (
    <>
      <WhatsNewCard />
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
            onClick={() => setActiveView("Models & language")}
            type="button"
          >
            Get a model
          </button>
        </div>
      ) : null}

      <section className="status-grid" aria-label="Current setup">
        <StatusCard
          action="Open Dictation settings"
          Icon={Gauge}
          label="Status"
          onAction={() => setActiveView("Dictation")}
          value={statusCardValue(appState.status)}
        />
        <StatusCard
          action="Open Microphone"
          Icon={Mic}
          label="Microphone"
          onAction={() => setActiveView("Microphone")}
          status={<span className="status-dot success" />}
          value={microphoneDisplayName(microphones, settings.selectedMicId)}
        />
        <StatusCard
          action="Open Models and language"
          Icon={Database}
          label="Model"
          onAction={() => setActiveView("Models & language")}
          value={modelName}
        />
        <StatusCard
          action="Open Dictation settings"
          Icon={Clipboard}
          label="After dictation"
          onAction={() => setActiveView("Dictation")}
          value={AFTER_DICTATION_LABELS[afterDictationOf(settings.outputMode)]}
        />
      </section>

      <StatsRow stats={stats} />

      {showLive ? (
        <article className="panel-card live-card">
          <div className="section-heading compact">
            <h2>Hearing you</h2>
          </div>
          <LiveTranscript text={liveText} />
        </article>
      ) : null}

      <section className="main-grid">
        <LastTranscriptCard
          clearing={actions.clearingLastTranscript}
          copying={actions.copyingLastTranscript}
          lastOutput={actions.lastOutput}
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
              All hotkeys
            </button>
          </div>
          <HotkeyList
            compact
            failedActions={failedHotkeys}
            only={shownHotkeys}
            settings={settings}
          />
          {failedHotkeys.size > 0 ? (
            <p className="hotkey-summary-note" role="status">
              {failedHotkeys.size === 1
                ? "One hotkey isn't working. Another app may use it."
                : `${failedHotkeys.size} hotkeys aren't working. Another app may use them.`}
            </p>
          ) : null}
        </article>
      </section>
    </>
  );
}

/** Actions whose shortcut is bound but not registered, so Home can flag a
 * conflict instead of listing a hotkey that does nothing. Reloads when the
 * bindings or the Local AI switch change. Status errors are left to the
 * Hotkeys page, which reports them in full. */
function useFailedHotkeys(
  hotkeysKey: string,
  aiEnabled: boolean,
): ReadonlySet<string> {
  const [failed, setFailed] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => {
    let cancelled = false;
    getHotkeyStatus()
      .then((status) => {
        if (!cancelled) {
          setFailed(
            new Set(
              status.bindings
                .filter((binding) => !binding.registered && !binding.disabledReason)
                .map((binding) => binding.action),
            ),
          );
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [hotkeysKey, aiEnabled]);
  return failed;
}
