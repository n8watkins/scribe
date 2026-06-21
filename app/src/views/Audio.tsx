import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Cpu, Mic, Play, RefreshCw } from "lucide-react";
import {
  commandErrorMessage,
  getTestClipAudio,
  listMicrophones,
  probeGpuDevices,
  recordTestClip,
  type AppSettings,
  type AppStateSnapshot,
  type AudioLevelEvent,
  type GpuProbe,
  type MicrophoneInfo,
  type RecordingErrorEvent,
} from "../backend";
import type { ViewActions } from "../types";
import { cleanMicName, selectedMicrophoneLabel } from "../lib/format";
import { EmptyState, InlineError } from "../components/feedback";
import { SectionPanel, SettingRow } from "../components/layout";
import { MsInput, Toggle } from "../components/primitives";
import { Waveform } from "../components/transcript";

export function AudioView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [microphones, setMicrophones] = useState<MicrophoneInfo[]>([]);
  const [microphonesLoading, setMicrophonesLoading] = useState(true);
  const [microphonesError, setMicrophonesError] = useState<string | null>(null);
  const [audioLevel, setAudioLevel] = useState(0);
  const [testingMic, setTestingMic] = useState(false);
  const [hasTestClip, setHasTestClip] = useState(false);
  const [playingTestClip, setPlayingTestClip] = useState(false);
  const [testClipError, setTestClipError] = useState<string | null>(null);
  const [gpuProbe, setGpuProbe] = useState<GpuProbe | null>(null);
  const [gpuProbing, setGpuProbing] = useState(true);

  const loadGpuProbe = useCallback(async () => {
    setGpuProbing(true);
    try {
      setGpuProbe(await probeGpuDevices());
    } catch {
      setGpuProbe(null);
    } finally {
      setGpuProbing(false);
    }
  }, []);

  useEffect(() => {
    void loadGpuProbe();
  }, [loadGpuProbe]);

  const loadMicrophones = useCallback(async () => {
    setMicrophonesLoading(true);
    setMicrophonesError(null);

    try {
      setMicrophones(await listMicrophones());
    } catch (error) {
      setMicrophonesError(commandErrorMessage(error));
    } finally {
      setMicrophonesLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadMicrophones();
  }, [loadMicrophones, settings.selectedMicId]);

  useEffect(() => {
    let disposed = false;
    let unlisteners: Array<() => void> = [];

    const setup = async () => {
      unlisteners = await Promise.all([
        listen<AudioLevelEvent>("audio://level", (event) => {
          setAudioLevel(Math.round(event.payload.level * 100));
        }),
        // Level events stop arriving once recording ends, which would otherwise
        // freeze the meter at its last value; zero it whenever we leave the
        // Recording state so the bar returns to rest.
        listen<AppStateSnapshot>("scribe:app-state", (event) => {
          if (event.payload.status !== "Recording") {
            setAudioLevel(0);
          }
        }),
        listen<RecordingErrorEvent>("audio://recording-error", (event) => {
          setMicrophonesError(event.payload.message);
        }),
      ]);

      if (disposed) {
        unlisteners.forEach((unlisten) => unlisten());
      }
    };

    void setup();

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  // Show the resolved default device in parens, e.g. "Default input device
  // (FIFINE Microphone)", and strip the Windows "Microphone (…)" wrapper.
  const defaultMic = microphones.find((microphone) => microphone.isDefault);
  const defaultMicLabel = defaultMic
    ? `Default input device (${cleanMicName(defaultMic.name)})`
    : "Default input device";
  const selectedMicrophone = settings.selectedMicId
    ? cleanMicName(selectedMicrophoneLabel(microphones, settings.selectedMicId))
    : defaultMicLabel;

  const handleRecordTestClip = useCallback(async () => {
    setTestingMic(true);
    setMicrophonesError(null);
    setTestClipError(null);

    try {
      await recordTestClip(1600);
      setHasTestClip(true);
      await loadMicrophones();
      await actions.refresh();
    } catch (error) {
      setMicrophonesError(commandErrorMessage(error));
    } finally {
      setTestingMic(false);
    }
  }, [actions, loadMicrophones]);

  const handlePlayTestClip = useCallback(async () => {
    setPlayingTestClip(true);
    setTestClipError(null);

    let url: string | null = null;
    try {
      const base64Wav = await getTestClipAudio();
      const bytes = Uint8Array.from(atob(base64Wav), (char) =>
        char.charCodeAt(0),
      );
      url = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
      const objectUrl = url;
      const audio = new Audio(objectUrl);
      const finish = () => {
        URL.revokeObjectURL(objectUrl);
        setPlayingTestClip(false);
      };
      audio.onended = finish;
      audio.onerror = finish;
      await audio.play();
    } catch (error) {
      // A rejected play() promise won't fire `onerror`, so revoke here too to
      // avoid leaking the object URL.
      if (url !== null) {
        URL.revokeObjectURL(url);
      }
      setTestClipError(commandErrorMessage(error));
      setPlayingTestClip(false);
    }
  }, []);

  return (
    <section className="split-grid">
      {/* Inlined (not <SectionPanel>) so the status badge can sit to the RIGHT
          of the "Input" title: the `.section-heading` is space-between, so the
          <h2> anchors left and the pill is pushed to the right edge. */}
      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Input</h2>
          <span
            className={
              microphonesError
                ? "pill error"
                : microphonesLoading
                  ? "pill pending"
                  : "pill ready"
            }
          >
            {microphonesError
              ? "Needs attention"
              : microphonesLoading
                ? "Scanning"
                : "Ready"}
          </span>
        </div>
        <div className="settings-list">
          <p className="muted audio-input-name" title={selectedMicrophone}>
            {selectedMicrophone}
          </p>

          {microphonesError ? (
            <InlineError message={microphonesError} onRetry={loadMicrophones} />
          ) : null}
          <Waveform />
          <div className="meter">
            <div style={{ width: `${Math.max(4, audioLevel)}%` }} />
          </div>

          <div className="control-grid single">
            <label>
              Microphone
              <select
                disabled={actions.savingSettings}
                onChange={(event) =>
                  actions.updateSettings({
                    selectedMicId:
                      event.currentTarget.value === "default"
                        ? null
                        : event.currentTarget.value,
                  })
                }
                value={settings.selectedMicId ?? "default"}
              >
                <option value="default">{defaultMicLabel}</option>
                {microphones.map((microphone) => (
                  <option
                    disabled={!microphone.isAvailable}
                    key={microphone.id}
                    value={microphone.id}
                  >
                    {microphone.name}
                    {microphone.isDefault ? " (default)" : ""}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div className="button-row">
            <button
              className="primary-button"
              disabled={
                testingMic ||
                actions.recordingBusy ||
                microphonesLoading ||
                Boolean(microphonesError)
              }
              onClick={() => void handleRecordTestClip()}
              type="button"
            >
              <Mic aria-hidden="true" size={16} />
              {testingMic ? "Testing..." : "Record test"}
            </button>
            <button
              className="secondary-button"
              disabled={!hasTestClip || playingTestClip || testingMic}
              onClick={() => void handlePlayTestClip()}
              type="button"
            >
              <Play aria-hidden="true" size={15} />
              {playingTestClip ? "Playing..." : "Play test"}
            </button>
          </div>
          {testClipError ? (
            <p className="field-error">{testClipError}</p>
          ) : null}
        </div>
      </article>

      <div className="stack">
        <SectionPanel title="Audio processing">
          <SettingRow
            description="Remove quiet space around speech."
            label="Silence trim"
          >
            <Toggle
              checked={settings.silenceTrimEnabled}
              disabled={actions.savingSettings}
              label="Silence trim"
              onChange={(silenceTrimEnabled) =>
                actions.updateSettings({ silenceTrimEnabled })
              }
            />
          </SettingRow>
          <SettingRow
            description="End toggle/manual recordings after a long pause. Off: keep listening through pauses; phrases still transcribe live in the background."
            label="Auto-stop on silence"
          >
            <Toggle
              checked={settings.silenceAutoStopEnabled}
              disabled={actions.savingSettings}
              label="Auto-stop on silence"
              onChange={(silenceAutoStopEnabled) =>
                actions.updateSettings({ silenceAutoStopEnabled })
              }
            />
          </SettingRow>
          <SettingRow
            description="Pause length that ends the recording, in milliseconds."
            label="Auto-stop pause (ms)"
          >
            <MsInput
              ariaLabel="Auto-stop pause length in milliseconds"
              disabled={
                actions.savingSettings || !settings.silenceAutoStopEnabled
              }
              min={1000}
              onCommit={(silenceAutoStopMs) =>
                actions.updateSettings({ silenceAutoStopMs })
              }
              value={settings.silenceAutoStopMs}
            />
          </SettingRow>
          <SettingRow
            description="Transcribe phrases in the background while you talk so text is ready the moment you stop."
            label="Live transcription"
          >
            <Toggle
              checked={settings.incrementalTranscriptionEnabled}
              disabled={actions.savingSettings}
              label="Live transcription"
              onChange={(incrementalTranscriptionEnabled) =>
                actions.updateSettings({ incrementalTranscriptionEnabled })
              }
            />
          </SettingRow>
          <SettingRow
            description="Split a phrase at a pause while you talk. Off: phrases split only when they get long (see below) — fewer sentence breaks, but text appears in larger jumps."
            label="Split on pause"
          >
            <Toggle
              checked={settings.segmentPauseEnabled}
              disabled={
                actions.savingSettings ||
                !settings.incrementalTranscriptionEnabled
              }
              label="Split on pause"
              onChange={(segmentPauseEnabled) =>
                actions.updateSettings({ segmentPauseEnabled })
              }
            />
          </SettingRow>
          <SettingRow
            description="How long a pause must be to split a phrase. Higher = fewer stray periods/commas when you pause mid-sentence, slightly more delay before text is ready (200–10000 ms)."
            label="Pause before splitting (ms)"
          >
            <MsInput
              ariaLabel="Pause length before splitting a phrase, in milliseconds"
              disabled={
                actions.savingSettings ||
                !settings.incrementalTranscriptionEnabled ||
                !settings.segmentPauseEnabled
              }
              max={10000}
              min={200}
              onCommit={(segmentPauseMs) =>
                actions.updateSettings({ segmentPauseMs })
              }
              value={settings.segmentPauseMs}
            />
          </SettingRow>
          <SettingRow
            description="Longest a single phrase can run before it is split anyway. Capped at 25 s because Whisper can only transcribe ~30 s at once (10000–25000 ms)."
            label="Max phrase length (ms)"
          >
            <MsInput
              ariaLabel="Maximum phrase length before forced split, in milliseconds"
              disabled={
                actions.savingSettings ||
                !settings.incrementalTranscriptionEnabled
              }
              max={25000}
              min={10000}
              onCommit={(segmentMaxMs) =>
                actions.updateSettings({ segmentMaxMs })
              }
              value={settings.segmentMaxMs}
            />
          </SettingRow>
          <SettingRow
            description="Ignore captures below this length, in milliseconds."
            label="Minimum duration (ms)"
          >
            <MsInput
              ariaLabel="Minimum duration in milliseconds"
              disabled={actions.savingSettings}
              min={1}
              onCommit={(minRecordingMs) =>
                actions.updateSettings({ minRecordingMs })
              }
              value={settings.minRecordingMs}
            />
          </SettingRow>
          <SettingRow
            description="Cap single dictation sessions, in milliseconds (600000 = 10 minutes)."
            label="Maximum duration (ms)"
          >
            <MsInput
              ariaLabel="Maximum duration in milliseconds"
              disabled={actions.savingSettings}
              min={settings.minRecordingMs}
              onCommit={(maxRecordingMs) =>
                actions.updateSettings({ maxRecordingMs })
              }
              value={settings.maxRecordingMs}
            />
          </SettingRow>
          <SettingRow description="Keep original clips for review." label="Save raw audio">
            <Toggle
              checked={settings.saveAudioClips}
              disabled={actions.savingSettings}
              label="Save raw audio"
              onChange={(saveAudioClips) =>
                actions.updateSettings({ saveAudioClips })
              }
            />
          </SettingRow>
        </SectionPanel>
        <SectionPanel
          icon={
            <span
              className={
                gpuProbing
                  ? "pill pending"
                  : gpuProbe?.available
                    ? "pill ready"
                    : "pill preserve"
              }
            >
              {gpuProbing
                ? "Detecting"
                : gpuProbe?.available
                  ? "GPU"
                  : "CPU"}
            </span>
          }
          title="GPU acceleration"
        >
          <SettingRow
            description="Use your GPU (Vulkan) to transcribe — a big speedup on the large models. Falls back to CPU automatically if the GPU is unavailable."
            label="Use GPU acceleration"
          >
            <Toggle
              checked={settings.gpuAcceleration !== "off"}
              disabled={actions.savingSettings}
              label="Use GPU acceleration"
              onChange={(on) =>
                actions.updateSettings({ gpuAcceleration: on ? "auto" : "off" })
              }
            />
          </SettingRow>
          <p
            className="muted"
            style={{ display: "flex", alignItems: "center", gap: 6 }}
          >
            <Cpu aria-hidden="true" size={14} />
            {gpuProbing
              ? "Detecting GPU…"
              : !gpuProbe?.probed
                ? "Couldn't detect — download a model to check your GPU."
                : !gpuProbe.available
                  ? "No Vulkan GPU found — transcription runs on CPU."
                  : `Detected: ${gpuProbe.devices
                      .map((device) => device.name)
                      .join(", ")}`}
          </p>
          {gpuProbe && gpuProbe.devices.length > 1 ? (
            <div className="control-grid single">
              <label>
                GPU device
                <select
                  disabled={
                    actions.savingSettings || settings.gpuAcceleration === "off"
                  }
                  onChange={(event) =>
                    actions.updateSettings({
                      gpuDeviceIndex:
                        event.currentTarget.value === "auto"
                          ? null
                          : Number(event.currentTarget.value),
                    })
                  }
                  value={settings.gpuDeviceIndex ?? "auto"}
                >
                  <option value="auto">Automatic (recommended)</option>
                  {gpuProbe.devices.map((device) => (
                    <option key={device.index} value={device.index}>
                      {device.name}
                      {device.integrated ? " (integrated)" : ""}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          ) : null}
        </SectionPanel>
        <SectionPanel
          icon={
            <span className={microphonesError ? "pill error" : "pill ready"}>
              {microphonesError ? "Unavailable" : "Available"}
            </span>
          }
          title="Device health"
        >
          {microphonesLoading ? (
            <div className="pending-panel">
              <RefreshCw aria-hidden="true" size={16} />
              <span>Scanning Windows input devices...</span>
            </div>
          ) : null}
          {!microphonesLoading && microphones.length === 0 ? (
            <EmptyState message="No input devices were reported by Windows. Connect a microphone and refresh." />
          ) : null}
          {!microphonesLoading && microphones.length > 0 ? (
            <div className="device-list">
              {microphones.map((microphone) => (
                <div className="device-row" key={microphone.id}>
                  <div>
                    <strong title={microphone.name}>{microphone.name}</strong>
                  </div>
                  <span
                    className={
                      microphone.isSelected
                        ? "pill selected"
                        : microphone.isDefault
                          ? "pill preserve"
                          : microphone.isAvailable
                            ? "pill ready"
                            : "pill error"
                    }
                  >
                    {microphone.isSelected
                      ? "Selected"
                      : microphone.isDefault
                        ? "Default"
                        : microphone.isAvailable
                          ? "Available"
                          : "Unavailable"}
                  </span>
                </div>
              ))}
            </div>
          ) : null}
        </SectionPanel>
      </div>
    </section>
  );
}
