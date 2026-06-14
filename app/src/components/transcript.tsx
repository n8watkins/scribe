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
  // single-line snippet already shows (multi-line, or longer than the cap).
  const canExpand = item.text.includes("\n") || item.text.trim().length > 140;

  const collapse = () => {
    // BUG fix: the expanded body scrolls internally, so reset it to the top
    // before collapsing — otherwise a later re-expand would start mid-transcript
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
      <div>
        {expanded && canExpand ? (
          <p className="transcript-full" ref={fullRef}>
            {item.text}{" "}
            <button
              className="text-button see-more-button"
              onClick={collapse}
              type="button"
            >
              See less
            </button>
          </p>
        ) : (
          // The whole collapsed snippet is the expand affordance: clicking
          // anywhere in the clamped text (or the trailing "…see more") opens it.
          <p
            className={
              canExpand
                ? "transcript-clamp is-expandable"
                : "transcript-clamp"
            }
            onClick={canExpand ? () => setExpanded(true) : undefined}
            role={canExpand ? "button" : undefined}
            tabIndex={canExpand ? 0 : undefined}
            title={item.text}
            onKeyDown={
              canExpand
                ? (event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setExpanded(true);
                    }
                  }
                : undefined
            }
          >
            {item.text}
            {canExpand ? (
              <span aria-hidden="true" className="see-more-inline">
                {" "}
                …see more
              </span>
            ) : null}
          </p>
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
        <span className="history-row-meta">
          {formatDateTime(item.createdAt)} · {transcriptMeta(item)}
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

const WAVEFORM_BARS = 13;
// Center bars react more than the edges, so the shape reads as a waveform.
// A smooth bell across all bars keeps the wider strip from looking blocky.
const WAVEFORM_ENVELOPE = Array.from({ length: WAVEFORM_BARS }, (_, i) => {
  const t = i / (WAVEFORM_BARS - 1); // 0..1 across the strip
  // Raised-cosine bell: 0.5 at the edges, 1 at the center.
  return 0.5 + 0.5 * Math.sin(Math.PI * t);
});
const WAVEFORM_REST = 0.16;
// Snappier than the old single 0.32 ease: jump UP fast on louder input so the
// strip tracks speech onsets, fall a touch slower so it doesn't strobe.
const WAVEFORM_ATTACK = 0.6;
const WAVEFORM_DECAY = 0.4;
// Horizontal hue progression so the whole strip reads as one multi-color
// gradient (cyan → blue → purple) rather than every bar an identical gradient.
const WAVEFORM_STOPS = ["#22d3ee", "#38bdf8", "#8b5cf6"] as const;

/** Interpolated bar color across the strip: cyan at the left edge, blue in the
 * middle, purple at the right, so the bars together form a horizontal gradient. */
function waveformColor(index: number): string {
  const t = WAVEFORM_BARS > 1 ? index / (WAVEFORM_BARS - 1) : 0;
  const scaled = t * (WAVEFORM_STOPS.length - 1);
  const lo = Math.min(Math.floor(scaled), WAVEFORM_STOPS.length - 2);
  const frac = scaled - lo;
  const mix = (a: number, b: number) => Math.round(a + (b - a) * frac);
  const parse = (hex: string) => [
    parseInt(hex.slice(1, 3), 16),
    parseInt(hex.slice(3, 5), 16),
    parseInt(hex.slice(5, 7), 16),
  ];
  const [r1, g1, b1] = parse(WAVEFORM_STOPS[lo]);
  const [r2, g2, b2] = parse(WAVEFORM_STOPS[lo + 1]);
  return `rgb(${mix(r1, r2)}, ${mix(g1, g2)}, ${mix(b1, b2)})`;
}

/** Live input visualizer: the bars react to the same `audio://level` stream the
 * floating pill uses, easing toward the mic level each frame (no per-frame React
 * state). They settle to a flat resting line whenever recording isn't active. */
export function Waveform() {
  const barsRef = useRef<Array<HTMLSpanElement | null>>([]);
  const levelRef = useRef(0);
  const displayRef = useRef<number[]>(
    new Array(WAVEFORM_BARS).fill(WAVEFORM_REST),
  );

  useEffect(() => {
    let disposed = false;
    let frame = 0;
    const unlisteners: Array<() => void> = [];

    const track = async () => {
      const stops = await Promise.all([
        listen<AudioLevelEvent>("audio://level", (event) => {
          levelRef.current = Math.max(0, Math.min(1, event.payload.rms));
        }),
        // Level events stop arriving when recording ends; zero the target so
        // the bars ease back to rest instead of freezing at the last value.
        listen<{ status: string }>("scribe:app-state", (event) => {
          if (event.payload.status !== "Recording") {
            levelRef.current = 0;
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
      const level = levelRef.current;
      const display = displayRef.current;
      for (let i = 0; i < WAVEFORM_BARS; i += 1) {
        // A little per-bar shimmer keeps it lively rather than a flat block.
        const jitter = level > 0.02 ? 0.82 + Math.random() * 0.36 : 1;
        const target = Math.min(
          1,
          WAVEFORM_REST + level * WAVEFORM_ENVELOPE[i] * jitter,
        );
        // Asymmetric ease: rise fast toward a louder target, settle a bit
        // slower, so the strip feels responsive without flickering.
        const ease =
          target > display[i] ? WAVEFORM_ATTACK : WAVEFORM_DECAY;
        display[i] += (target - display[i]) * ease;
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
      {Array.from({ length: WAVEFORM_BARS }, (_, i) => (
        <span
          key={i}
          ref={(element) => {
            barsRef.current[i] = element;
          }}
          style={{ background: waveformColor(i) }}
        />
      ))}
    </div>
  );
}
