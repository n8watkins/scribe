import { useState } from "react";
import { AlertCircle, Copy, Download, Eraser, Mic, Play, Square } from "lucide-react";
import {
  commandErrorMessage,
  saveTextFile,
  transcribeFile,
  type DashboardData,
  type PartialTranscriptEvent,
  type TranscribeFileResult,
} from "../backend";
import type { ViewActions } from "../types";
import {
  canStartRecording,
  clipboardStatus,
  formatHotkey,
  formatMsReadable,
  outputModeOptions,
  pasteMethodOptions,
  recordingStageTitle,
} from "../lib/format";
import { SegmentedControl, StatePill } from "../components/primitives";
import {
  LastTranscriptCard,
  LiveTranscript,
  Waveform,
} from "../components/transcript";

export function TranscribeView({
  actions,
  data,
  liveTranscript,
}: {
  actions: ViewActions;
  data: DashboardData;
  liveTranscript: PartialTranscriptEvent | null;
}) {
  const { appState, lastTranscript, settings } = data;
  const liveText = liveTranscript?.text.trim() ?? "";
  const showLiveTranscript =
    liveText.length > 0 &&
    (appState.status === "Recording" ||
      appState.status === "Stopping" ||
      appState.status === "Transcribing");

  return (
    <>
    <section className="split-grid">
      <article className="buffer-card">
        <div className="section-heading">
          <div>
            <p className="eyebrow">Recording</p>
            <h2>Push-to-talk capture</h2>
          </div>
          <StatePill appState={appState} />
        </div>

        <div className="recording-stage">
          <Waveform />
          <div>
            <strong>{recordingStageTitle(appState.status)}</strong>
            <p className="muted">
              Hold {formatHotkey(settings.hotkeys.holdToTalk)} or use toggle
              mode. Audio is transcribed locally.
            </p>
            {showLiveTranscript ? <LiveTranscript text={liveText} /> : null}
          </div>
        </div>

        <div className="button-row">
          <button
            className="primary-button"
            disabled={actions.recordingBusy || !canStartRecording(data)}
            onClick={() => void actions.startRecording()}
            type="button"
          >
            <Mic aria-hidden="true" size={16} />
            {actions.recordingBusy ? "Working..." : "Start"}
          </button>
          {appState.status === "Recording" ? (
            <>
              <button
                className="secondary-button"
                disabled={actions.recordingBusy}
                onClick={() => void actions.stopRecording()}
                type="button"
              >
                <Square aria-hidden="true" size={15} />
                Stop
              </button>
              <button
                className="ghost-button"
                disabled={actions.recordingBusy}
                onClick={() => void actions.cancelRecording()}
                type="button"
              >
                <Eraser aria-hidden="true" size={15} />
                Discard
              </button>
            </>
          ) : null}
        </div>
      </article>

      <div className="stack">
        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Output behavior</h2>
            <span className="pill preserve">{clipboardStatus(settings)}</span>
          </div>
          <SegmentedControl
            disabled={actions.savingSettings}
            onChange={(outputMode) => actions.updateSettings({ outputMode })}
            options={outputModeOptions}
            selected={settings.outputMode}
          />
        </article>

        <article className="panel-card">
          <div className="section-heading compact">
            <h2>Paste method</h2>
          </div>
          <SegmentedControl
            disabled={actions.savingSettings}
            onChange={(pasteMethod) => actions.updateSettings({ pasteMethod })}
            options={pasteMethodOptions}
            selected={settings.pasteMethod}
          />
        </article>

        <LastTranscriptCard
          clearing={actions.clearingLastTranscript}
          compact
          copying={actions.copyingLastTranscript}
          onClear={actions.clearLastTranscript}
          onCopy={actions.copyLastTranscript}
          onPaste={actions.pasteLastTranscript}
          pasting={actions.pastingLastTranscript}
          settings={settings}
          transcript={lastTranscript}
        />
      </div>
    </section>

    <FileTranscribeCard />
    </>
  );
}

function FileTranscribeCard() {
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<TranscribeFileResult | null>(null);
  // Path the current result came from, so Save targets it even if the input
  // is edited afterwards.
  const [resultPath, setResultPath] = useState("");
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);

  const handleTranscribe = async () => {
    const trimmed = path.trim();
    if (!trimmed || busy) {
      return;
    }

    setBusy(true);
    setError(null);
    setResult(null);
    setSavedPath(null);
    setCopied(false);
    try {
      const next = await transcribeFile(trimmed);
      setResult(next);
      setResultPath(trimmed);
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const handleCopy = async () => {
    if (!result) {
      return;
    }
    try {
      await navigator.clipboard.writeText(result.text);
      setCopied(true);
    } catch (cause) {
      setError(commandErrorMessage(cause));
    }
  };

  const handleSave = async () => {
    if (!result || saving) {
      return;
    }
    setSaving(true);
    setError(null);
    try {
      setSavedPath(await saveTextFile(resultPath, result.text));
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  return (
    <article className="panel-card file-transcribe-card">
      <div className="section-heading compact">
        <h2>Transcribe a file</h2>
        {result ? (
          <span className="pill preserve">
            Done in {formatMsReadable(result.latencyMs)}
          </span>
        ) : null}
      </div>
      <p className="muted">
        Transcribe an existing audio or video file locally. WAV, MP3, FLAC,
        and OGG work out of the box; other formats (MP4, MKV, M4A, ...) need
        ffmpeg on PATH.
      </p>
      <div className="toolbar-row">
        <input
          aria-label="Audio or video file path"
          disabled={busy}
          onChange={(event) => setPath(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              void handleTranscribe();
            }
          }}
          placeholder="C:\Users\you\Videos\meeting.mp4"
          spellCheck={false}
          type="text"
          value={path}
        />
        <button
          className="primary-button"
          disabled={busy || !path.trim()}
          onClick={() => void handleTranscribe()}
          type="button"
        >
          <Play aria-hidden="true" size={15} />
          {busy ? "Transcribing..." : "Transcribe"}
        </button>
      </div>
      {busy ? (
        <p className="muted">Transcribing — large files can take a while.</p>
      ) : null}
      {error ? (
        <div className="inline-error">
          <AlertCircle aria-hidden="true" size={16} />
          <span>{error}</span>
        </div>
      ) : null}
      {result ? (
        <>
          <textarea
            aria-label="File transcription result"
            readOnly
            rows={6}
            value={result.text}
          />
          <div className="button-row">
            <button
              className="secondary-button"
              onClick={() => void handleCopy()}
              type="button"
            >
              <Copy aria-hidden="true" size={15} />
              {copied ? "Copied" : "Copy"}
            </button>
            <button
              className="secondary-button"
              disabled={saving}
              onClick={() => void handleSave()}
              type="button"
            >
              <Download aria-hidden="true" size={15} />
              {saving ? "Saving..." : "Save as .txt next to the source file"}
            </button>
          </div>
          {savedPath ? <p className="muted">Saved to {savedPath}</p> : null}
        </>
      ) : null}
    </article>
  );
}
