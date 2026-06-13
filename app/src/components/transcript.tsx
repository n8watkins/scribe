import { useEffect, useRef } from "react";
import {
  ClipboardPaste,
  Copy,
  Eraser,
  Play,
  Sparkles,
  Square,
  Trash2,
} from "lucide-react";
import type { Transcript } from "../backend";
import {
  formatCount,
  formatDateTime,
  formatDuration,
  transcriptMeta,
  transcriptTitle,
} from "../lib/format";
import { EmptyState } from "./feedback";
import { IconButton } from "./primitives";

export function LastTranscriptCard({
  clearing,
  compact = false,
  copying,
  onClear,
  onCopy,
  onPaste,
  pasting,
  transcript,
}: {
  clearing: boolean;
  compact?: boolean;
  copying: boolean;
  onClear: () => Promise<void>;
  onCopy: () => Promise<void>;
  onPaste: () => Promise<void>;
  pasting: boolean;
  transcript: Transcript | null;
}) {
  const hasTranscript = Boolean(transcript);
  const outputBusy = clearing || copying || pasting;

  return (
    <article className={compact ? "panel-card" : "buffer-card"}>
      <div className="section-heading">
        <div>
          <p className="eyebrow">Last Transcript Buffer</p>
          <h2>{hasTranscript ? "Ready to insert later" : "No transcript stored"}</h2>
        </div>
        <span className="pill preserve">Clipboard Preserved</span>
      </div>

      {transcript ? (
        <>
          <p
            className={
              compact ? "transcript-text compact-text" : "transcript-text"
            }
          >
            {transcript.text}
          </p>

          <div className="metadata-row">
            <span>{formatCount(transcript.wordCount, "word")}</span>
            <span>{formatCount(transcript.characterCount, "char")}</span>
            <span>{formatDuration(transcript.durationMs)}</span>
            <span>{transcript.modelId ?? "No model recorded"}</span>
            <span>{formatDateTime(transcript.createdAt)}</span>
          </div>
        </>
      ) : (
        <EmptyState message="Complete a transcription to populate the buffer. Clipboard remains untouched." />
      )}

      <div className="button-row">
        <IconButton
          disabled={!hasTranscript || outputBusy}
          label={pasting ? "Inserting..." : "Insert into focused app"}
          onClick={() => void onPaste()}
        >
          <ClipboardPaste aria-hidden="true" size={15} />
        </IconButton>
        <IconButton
          disabled={!hasTranscript || outputBusy}
          label={copying ? "Copying..." : "Copy to clipboard"}
          onClick={() => void onCopy()}
        >
          <Copy aria-hidden="true" size={15} />
        </IconButton>
        <IconButton
          danger
          disabled={!hasTranscript || outputBusy}
          label={clearing ? "Clearing..." : "Clear buffer"}
          onClick={() => void onClear()}
        >
          <Eraser aria-hidden="true" size={15} />
        </IconButton>
      </div>
    </article>
  );
}

export function TranscriptRow({
  busy = false,
  item,
  onAnalyze,
  onCopy,
  onDelete,
  onPaste,
  onPlay,
  playing = false,
}: {
  busy?: boolean;
  item: Transcript;
  onAnalyze?: (id: string) => Promise<void>;
  onCopy?: (id: string) => Promise<void>;
  onDelete?: (id: string) => Promise<void>;
  onPaste?: (id: string) => Promise<void>;
  onPlay?: (id: string) => Promise<void>;
  playing?: boolean;
}) {
  return (
    <div className="history-row">
      <div>
        <strong>{transcriptTitle(item)}</strong>
        <p title={item.text}>{item.text}</p>
        {item.analysis ? (
          <div className="note-analysis">
            <span className="note-analysis-label">
              <Sparkles aria-hidden="true" size={11} />
              {item.analysisModel ?? "local LLM"}
            </span>
            <p>{item.analysis}</p>
          </div>
        ) : null}
        <span>{transcriptMeta(item)}</span>
      </div>
      <div className="row-actions">
        {onAnalyze ? (
          <IconButton
            disabled={busy}
            label={
              busy
                ? "Analyzing..."
                : item.analysis
                  ? "Re-run notes analysis"
                  : "Analyze with local LLM"
            }
            onClick={() => void onAnalyze(item.id)}
          >
            <Sparkles aria-hidden="true" size={15} />
          </IconButton>
        ) : null}
        {item.audioPath ? (
          <IconButton
            disabled={busy || !onPlay}
            label={playing ? "Stop playback" : "Play recording"}
            onClick={() => void onPlay?.(item.id)}
          >
            {playing ? (
              <Square aria-hidden="true" size={15} />
            ) : (
              <Play aria-hidden="true" size={15} />
            )}
          </IconButton>
        ) : null}
        <IconButton
          disabled={busy || !onCopy}
          label="Copy to clipboard"
          onClick={() => void onCopy?.(item.id)}
        >
          <Copy aria-hidden="true" size={15} />
        </IconButton>
        <IconButton
          disabled={busy || !onPaste}
          label={busy ? "Working..." : "Insert into focused app"}
          onClick={() => void onPaste?.(item.id)}
        >
          <ClipboardPaste aria-hidden="true" size={15} />
        </IconButton>
        <IconButton
          danger
          disabled={busy || !onDelete}
          label="Delete transcript"
          onClick={() => void onDelete?.(item.id)}
        >
          <Trash2 aria-hidden="true" size={15} />
        </IconButton>
      </div>
    </div>
  );
}

export function LiveTranscript({ text }: { text: string }) {
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Keep the newest words visible as the transcript grows.
  useEffect(() => {
    const node = scrollRef.current;
    if (node) {
      node.scrollTop = node.scrollHeight;
    }
  }, [text]);

  return (
    <div className="live-transcript">
      <span aria-hidden="true" className="live-transcript-dot" />
      <div aria-live="polite" className="live-transcript-text" ref={scrollRef}>
        {text}
      </div>
    </div>
  );
}

export function Waveform() {
  return (
    <div className="recording-visual" aria-hidden="true">
      <span />
      <span />
      <span />
      <span />
      <span />
      <span />
      <span />
    </div>
  );
}
