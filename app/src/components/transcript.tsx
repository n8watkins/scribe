import { useEffect, useRef, useState } from "react";
import {
  ClipboardPaste,
  Copy,
  Eraser,
  ExternalLink,
  Play,
  Sparkles,
  Square,
  Trash2,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import type { AppSettings, AudioLevelEvent, Transcript } from "../backend";
import {
  clipboardStatus,
  formatCount,
  formatDateTime,
  formatDuration,
  transcriptMeta,
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
  settings,
  transcript,
}: {
  clearing: boolean;
  compact?: boolean;
  copying: boolean;
  onClear: () => Promise<void>;
  onCopy: () => Promise<void>;
  onPaste: () => Promise<void>;
  pasting: boolean;
  settings: AppSettings;
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
        <span className="pill preserve">{clipboardStatus(settings)}</span>
      </div>

      {transcript ? (
        <>
          <p
            className={
              compact
                ? "transcript-text compact-text"
                : "transcript-text buffer-preview"
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
  onOpenExternally,
  onPaste,
  onPlay,
  onToggleSelect,
  playing = false,
  selected = false,
}: {
  busy?: boolean;
  item: Transcript;
  onAnalyze?: (id: string) => Promise<void>;
  onCopy?: (id: string) => Promise<void>;
  onDelete?: (id: string) => Promise<void>;
  onOpenExternally?: (id: string) => Promise<void>;
  onPaste?: (id: string) => Promise<void>;
  onPlay?: (id: string) => Promise<void>;
  onToggleSelect?: (id: string) => void;
  playing?: boolean;
  selected?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const fullRef = useRef<HTMLParagraphElement | null>(null);
  // Only offer the toggle when there is genuinely more to reveal than the
  // 3-line clamp already shows (multi-line, or longer than the cap).
  const canExpand = item.text.includes("\n") || item.text.trim().length > 200;

  const collapse = () => {
    // The expanded body scrolls internally, so reset it to the top before
    // collapsing — otherwise a later re-expand would start mid-transcript
    // instead of at the beginning.
    if (fullRef.current) {
      fullRef.current.scrollTop = 0;
    }
    setExpanded(false);
  };

  return (
    <div className={selected ? "history-row is-selected" : "history-row"}>
      {onToggleSelect ? (
        <input
          aria-label="Select transcript for combine"
          checked={selected}
          className="history-row-select"
          onChange={() => onToggleSelect(item.id)}
          type="checkbox"
        />
      ) : null}
      <div className="history-row-body">
        {/* Date leads the row, muted/secondary, above the transcript text. */}
        <span className="history-row-date">{formatDateTime(item.createdAt)}</span>
        {expanded && canExpand ? (
          <p className="transcript-full" ref={fullRef}>
            {item.text}
          </p>
        ) : (
          <p className="transcript-clamp">{item.text}</p>
        )}
        {item.analysis ? (
          <div className="note-analysis">
            <span className="note-analysis-label">
              <Sparkles aria-hidden="true" size={11} />
              {item.analysisModel ?? "local LLM"}
            </span>
            <p>{item.analysis}</p>
          </div>
        ) : null}
        {/* Meta line: existing details (words · model · duration) with the
            See more/less toggle pinned to the RIGHT. This row never moves or
            mutates when the text expands — only the text area above grows. */}
        <span className="history-row-meta">
          <span className="history-row-meta-text">{transcriptMeta(item)}</span>
          {canExpand ? (
            <button
              className="text-button see-toggle"
              onClick={() => (expanded ? collapse() : setExpanded(true))}
              type="button"
            >
              {expanded ? "See less" : "See more"}
            </button>
          ) : null}
        </span>
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
        {onOpenExternally ? (
          <IconButton
            disabled={busy}
            label="Open in external editor"
            onClick={() => void onOpenExternally(item.id)}
          >
            <ExternalLink aria-hidden="true" size={15} />
          </IconButton>
        ) : null}
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

// The in-app waveform is the floating pill's `Visualizer` engine verbatim (see
// PillApp.tsx): a rolling history of recent mic levels scrolls left as new
// samples enter on the right, instead of every bar pulsing off the current
// level. Keep these constants in lockstep with the pill so both read identically.
const BAR_COUNT = 15;
// Bars never collapse to nothing; silence reads as a low resting line.
const BAR_MIN_SCALE = 0.14;
const BAR_ATTACK = 0.45;
const BAR_DECAY = 0.16;
// The waveform tapers toward both ends so the edges stay quieter than the
// center. Ends of the envelope reach this fraction of the center's height.
const BAR_EDGE_GAIN = 0.3;
// Normal speech RMS sits around 0.03-0.15, so raw values barely move the bars.
// Normalize against this ceiling before the perceptual curve.
const RMS_CEILING = 0.07;

/**
 * Raised-cosine (Hann-style) envelope over the bar index: ~1.0 at the center,
 * falling smoothly to BAR_EDGE_GAIN at both ends. Computed once from the bar
 * count and applied to each bar's dynamic height so the waveform fades out at
 * its edges. A single bar (or no bars) degrades to a flat 1.0.
 */
const BAR_ENVELOPE: number[] = Array.from({ length: BAR_COUNT }, (_, i) => {
  if (BAR_COUNT <= 1) {
    return 1;
  }
  const hann = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (BAR_COUNT - 1));
  return BAR_EDGE_GAIN + (1 - BAR_EDGE_GAIN) * hann;
});

/** Maps raw mic RMS to a 0..1 perceptual level that visibly animates bars. */
function perceptualLevel(rms: number): number {
  return Math.sqrt(Math.min(1, Math.max(0, rms) / RMS_CEILING));
}

/**
 * Live input visualizer: the exact rolling-waveform engine the floating pill
 * uses (PillApp `Visualizer`). New mic samples enter on the right and scroll
 * left as the history shifts; a requestAnimationFrame loop eases the displayed
 * bars toward their targets by mutating `transform: scaleY(...)` directly (no
 * per-frame React state). The history drains back to rest whenever recording
 * isn't active.
 */
export function Waveform() {
  const barsRef = useRef<Array<HTMLSpanElement | null>>([]);
  const historyRef = useRef<number[]>(new Array(BAR_COUNT).fill(0));
  const displayRef = useRef<number[]>(new Array(BAR_COUNT).fill(0));

  useEffect(() => {
    let disposed = false;
    let frame = 0;
    const unlisteners: Array<() => void> = [];

    const track = async () => {
      const stops = await Promise.all([
        listen<AudioLevelEvent>("audio://level", (event) => {
          const history = historyRef.current;
          history.push(perceptualLevel(event.payload.rms));
          if (history.length > BAR_COUNT) {
            history.shift();
          }
        }),
        // Level events stop arriving when recording ends; clear the history so
        // the bars scroll back to rest instead of freezing at the last values.
        listen<{ status: string }>("scribe:app-state", (event) => {
          if (event.payload.status !== "Recording") {
            historyRef.current.fill(0);
          }
        }),
      ]);
      if (disposed) {
        stops.forEach((stop) => stop());
      } else {
        unlisteners.push(...stops);
      }
    };

    const tick = () => {
      const history = historyRef.current;
      const display = displayRef.current;
      for (let i = 0; i < BAR_COUNT; i += 1) {
        // The edge taper scales only the moving part, so the resting line
        // (BAR_MIN_SCALE) stays flat across every bar.
        const target =
          BAR_MIN_SCALE +
          (history[i] ?? 0) * BAR_ENVELOPE[i] * (1 - BAR_MIN_SCALE);
        const current = display[i];
        const rate = target > current ? BAR_ATTACK : BAR_DECAY;
        display[i] = current + (target - current) * rate;
        const bar = barsRef.current[i];
        if (bar) {
          bar.style.transform = `scaleY(${display[i].toFixed(3)})`;
        }
      }
      frame = requestAnimationFrame(tick);
    };

    void track();
    frame = requestAnimationFrame(tick);

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      unlisteners.forEach((stop) => stop());
    };
  }, []);

  return (
    <div className="recording-visual" aria-hidden="true">
      {Array.from({ length: BAR_COUNT }, (_, i) => (
        <span
          key={i}
          ref={(element) => {
            barsRef.current[i] = element;
          }}
        />
      ))}
    </div>
  );
}
