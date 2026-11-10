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
  resolveMicrophone,
  statusCardValue,
} from "../lib/format";
import { HotkeyList, StatsRow, StatusCard } from "../components/layout";
import { LastTranscriptCard, LiveTranscript } from "../components/transcript";
import { WhatsNewCard } from "../components/ReleaseNotes";
import { SetupStrip } from "../components/SetupStrip";
import { isParakeetModel } from "../lib/engine";
import { insertMethodLabel, usePastePermission } from "../lib/pastePermission";
import { engineStateLabel, useEngineStatus } from "../lib/engineStatus";

/** The hotkeys Home always lists; any other hotkey shows up only when it
 * failed to register, so a conflict is never hidden. */
const HOME_HOTKEYS = [
  "holdToTalk",
  "toggleDictation",
  "pasteLastTranscript",
  "discardDictation",
  "openDashboard",
  "transformSelection",
];

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
  const microphone = resolveMicrophone(microphones, settings.selectedMicId);
  const microphoneFellBack =
    microphone.kind === "device" && microphone.fellBack;
  // Amber only when something needs action: no microphone at all. A saved
  // microphone that is unplugged is fine, Scribe records on the default.
  const microphoneDot =
    microphone.kind === "loading"
      ? "status-dot"
      : microphone.kind === "none"
        ? "status-dot warning"
        : "status-dot success";
  const hotkeyHealth = useHotkeyHealth(
    JSON.stringify(settings.hotkeys),
    settings.notesAnalysisEnabled,
  );
  const failedHotkeys = hotkeyHealth.failed;
  const shownHotkeys = new Set([...HOME_HOTKEYS, ...failedHotkeys]);
  const { permission } = usePastePermission();
  const after = afterDictationOf(settings.outputMode);
  // On Linux, "Insert at cursor" names the way it really happens: through
  // the Wayland portal, X11 keystrokes, or not at all (copied instead).
  const afterValue =
    (after === "insert" ? insertMethodLabel(permission) : null) ??
    AFTER_DICTATION_LABELS[after];
  const engineStatus = useEngineStatus();
  const engineMatches =
    engineStatus !== null &&
    engineStatus.modelId === settings.selectedModelId;
  const modelName =
    (engineMatches ? engineStatus.modelName : null) ??
    models?.find((model) => model.id === settings.selectedModelId)?.name ??
    settings.selectedModelId ??
    "Choose a model";
  const engineName = isParakeetModel(settings.selectedModelId)
    ? "Parakeet"
    : "Whisper";
  const modelDetail =
    modelReady && engineMatches
      ? `${engineName} · ${engineStateLabel(engineStatus)}`
      : engineName;
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
              Transcription needs a local speech model on this device.
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
          detail={microphoneFellBack ? "Saved mic not connected" : undefined}
          quiet={
            microphoneFellBack && microphone.kind === "device"
              ? {
                  label: "Make this my microphone",
                  onClick: () =>
                    actions.updateSettings({
                      selectedMicId: microphone.microphone.id,
                    }),
                }
              : undefined
          }
          Icon={Mic}
          label="Microphone"
          onAction={() => setActiveView("Microphone")}
          status={<span className={microphoneDot} />}
          value={microphoneDisplayName(microphone)}
        />
        <StatusCard
          action="Open Models and language"
          Icon={Database}
          detail={modelDetail}
          label="Model"
          onAction={() => setActiveView("Models & language")}
          value={modelName}
        />
        <StatusCard
          action="Open Dictation settings"
          Icon={Clipboard}
          label="After dictation"
          onAction={() => setActiveView("Dictation")}
          value={afterValue}
        />
      </section>

      <SetupStrip
        actions={actions}
        settings={settings}
        setActiveView={setActiveView}
      />

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
            systemTriggers={
              hotkeyHealth.managedByDesktop
                ? hotkeyHealth.systemTriggers
                : undefined
            }
          />
          {failedHotkeys.size > 0 ? (
            <p className="hotkey-summary-note" role="status">
              {failedHotkeysNote(
                failedHotkeys.size,
                hotkeyHealth.managedByDesktop,
              )}
            </p>
          ) : null}
        </article>
      </section>
    </>
  );
}

type HotkeyHealth = {
  /** Actions whose shortcut is bound but not registered. */
  failed: ReadonlySet<string>;
  /** The desktop's GlobalShortcuts portal owns the keys (Wayland). */
  managedByDesktop: boolean;
  /** The keys the desktop assigned, by action, when it owns them. */
  systemTriggers: Readonly<Record<string, string>>;
};

const EMPTY_HEALTH: HotkeyHealth = {
  failed: new Set(),
  managedByDesktop: false,
  systemTriggers: {},
};

/** Actions whose shortcut is bound but not registered, so Home can flag a
 * conflict instead of listing a hotkey that does nothing, and the keys the
 * desktop assigned when it owns the hotkeys. Reloads when the bindings or
 * the Local AI switch change. Status errors are left to the Hotkeys page,
 * which reports them in full. */
function useHotkeyHealth(
  hotkeysKey: string,
  aiEnabled: boolean,
): HotkeyHealth {
  const [health, setHealth] = useState<HotkeyHealth>(EMPTY_HEALTH);
  useEffect(() => {
    let cancelled = false;
    getHotkeyStatus()
      .then((status) => {
        if (cancelled) {
          return;
        }
        const systemTriggers: Record<string, string> = {};
        for (const binding of status.bindings) {
          if (binding.systemTrigger) {
            systemTriggers[binding.action] = binding.systemTrigger;
          }
        }
        setHealth({
          failed: new Set(
            status.bindings
              .filter((binding) => !binding.registered && !binding.disabledReason)
              .map((binding) => binding.action),
          ),
          managedByDesktop: status.backend === "portal",
          systemTriggers,
        });
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [hotkeysKey, aiEnabled]);
  return health;
}

/** Home's one-line summary of hotkeys that do nothing. */
function failedHotkeysNote(count: number, managedByDesktop: boolean): string {
  if (managedByDesktop) {
    return count === 1
      ? "One hotkey has no keys from your desktop yet. See Hotkeys."
      : `${count} hotkeys have no keys from your desktop yet. See Hotkeys.`;
  }
  return count === 1
    ? "One hotkey isn't working. Another app may use it."
    : `${count} hotkeys aren't working. Another app may use them.`;
}
