import { useState } from "react";
import { AlertCircle, Copy, Download, FileAudio, Play } from "lucide-react";
import {
  commandErrorMessage,
  saveTextFile,
  transcribeFile,
  type TranscribeFileResult,
} from "../backend";
import { formatMsReadable } from "../lib/format";
import "./transcribe.css";

/** The "Transcribe a file" page: transcribe an existing audio or video file
 * on this computer. Owns its own state and talks to the backend directly. */
export function FileTranscribeCard() {
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
    <article className="panel-card file-transcribe-card" data-setting="Transcribe a file">
      <div className="section-heading compact">
        <h2>
          <FileAudio aria-hidden="true" size={16} />
          Transcribe a file
        </h2>
        {result ? (
          <span className="pill preserve">
            Done in {formatMsReadable(result.latencyMs)}
          </span>
        ) : null}
      </div>
      <p className="muted">
        Turn an audio or video file into text, on this computer. WAV, MP3, FLAC
        and OGG work as they are; other formats such as MP4, MKV and M4A need
        ffmpeg installed.
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
        <p className="muted">Transcribing. Large files can take a while.</p>
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
            className="transcribe-file-result"
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
              {saving ? "Saving..." : "Save as .txt next to the file"}
            </button>
          </div>
          {savedPath ? <p className="muted">Saved to {savedPath}</p> : null}
        </>
      ) : null}
    </article>
  );
}
