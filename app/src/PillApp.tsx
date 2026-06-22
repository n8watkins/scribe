import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  availableMonitors,
  currentMonitor,
  getCurrentWindow,
  LogicalSize,
  PhysicalPosition,
} from "@tauri-apps/api/window";
import { Check } from "lucide-react";
import {
  copyTranscript,
  getAppState,
  getSettings,
  stopRecording,
  transcribeRecording,
  updateSettings,
  type AppSettings,
  type AppStateSnapshot,
  type AppStatus,
  type AudioLevelEvent,
  type DictationResult,
  type PartialTranscriptEvent,
  type PillDisplayMode,
} from "./backend";
import "./pill.css";

const VISIBLE_STATUSES: ReadonlySet<AppStatus> = new Set([
  "Recording",
  "Stopping",
  "Transcribing",
  "Pasting",
  "Ready",
  "Error",
]);

const READY_HIDE_DELAY_MS = 2000;
const CONFIRM_HIDE_DELAY_MS = 8000;
const COPIED_RESET_MS = 1500;
const MOVE_PERSIST_DEBOUNCE_MS = 600;
const BOTTOM_MARGIN_PX = 90;
const DRAG_THRESHOLD_PX = 4;

const BAR_COUNT = 15;

/** Converts a #rrggbb hex to an rgba() string at the given alpha so the pill
 * background stays slightly translucent over the desktop. Returns the input
 * unchanged if it is not a 6-digit hex. */
function hexWithAlpha(hex: string, alpha: number): string {
  const match = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!match) {
    return hex;
  }
  const value = parseInt(match[1], 16);
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`;
}
// Bars never collapse to nothing; silence reads as a low resting line.
const BAR_MIN_SCALE = 0.14;
const BAR_ATTACK = 0.45;
const BAR_DECAY = 0.16;
// The waveform tapers toward both ends so the edges stay quieter than the
// center. Ends of the envelope reach this fraction of the center's height.
const BAR_EDGE_GAIN = 0.3;

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
  // Hann window: 0 at the ends, 1 at the center. Rescale its [0,1] range into
  // [BAR_EDGE_GAIN, 1] so the edges keep BAR_EDGE_GAIN of the center height.
  const hann = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (BAR_COUNT - 1));
  return BAR_EDGE_GAIN + (1 - BAR_EDGE_GAIN) * hann;
});
// Normal speech RMS sits around 0.03-0.15, so raw values barely move the
// bars. Normalize against this ceiling before the perceptual curve.
const RMS_CEILING = 0.07;

/**
 * The pill window renders one of three physical layouts; each maps to a fixed
 * logical window size so nothing is clipped (the CSS in pill.css must fit
 * inside these).
 */
type PillLayout = "dot" | "bar" | "text";

const LAYOUT_SIZES: Record<PillLayout, { width: number; height: number }> = {
  dot: { width: 58, height: 56 },
  bar: { width: 158, height: 56 },
  text: { width: 320, height: 80 },
};

/** Text mode starts at the compact LAYOUT_SIZES.text height and grows with
 * the transcript (bottom edge anchored) up to this cap; older lines then
 * clip off the top. */
const TEXT_MAX_HEIGHT = 150;

function pillLayout(
  mode: PillDisplayMode,
  status: AppStatus | null,
  recording: boolean,
  confirming: boolean,
): PillLayout {
  // Text mode also covers the brief stop/transcribe/paste window so the
  // grown pill doesn't bounce through the bar size before confirming.
  const pending =
    status === "Stopping" || status === "Transcribing" || status === "Pasting";
  if (
    mode === "visualizer_with_text" &&
    (recording || confirming || pending)
  ) {
    return "text";
  }
  // The confirmation header (check + "Transcribed" + Copy) needs the full
  // bar width even in dot mode.
  if (mode === "dot" && !confirming) {
    return "dot";
  }
  return "bar";
}

/** Maps raw mic RMS to a 0..1 perceptual level that visibly animates bars. */
function perceptualLevel(rms: number) {
  return Math.sqrt(Math.min(1, Math.max(0, rms) / RMS_CEILING));
}

/**
 * True when the physical point (x, y) lands within some connected monitor's
 * bounds. Windows parks a minimized or hidden window at (-32000, -32000), and
 * unplugging a monitor leaves saved coordinates pointing at nothing; in both
 * cases the pill would be restored off-screen and look like it vanished. We use
 * this to refuse to persist — or restore to — a position the pill could never
 * be seen at. Defaults to true when monitor info is unavailable (e.g. outside
 * Tauri) so it never blocks a legitimate position.
 */
async function positionOnScreen(x: number, y: number): Promise<boolean> {
  try {
    const monitors = await availableMonitors();
    if (monitors.length === 0) {
      return true;
    }
    return monitors.some(
      ({ position, size }) =>
        x >= position.x &&
        y >= position.y &&
        x < position.x + size.width &&
        y < position.y + size.height,
    );
  } catch {
    return true;
  }
}

function pillTone(status: AppStatus) {
  switch (status) {
    case "Recording":
      return "recording";
    case "Stopping":
    case "Transcribing":
    case "Pasting":
      return "pending";
    case "Ready":
      return "ready";
    case "Error":
      return "error";
    default:
      return "idle";
  }
}

function pillLabel(appState: AppStateSnapshot): string {
  switch (appState.status) {
    case "Stopping":
      return "Saving...";
    case "Transcribing":
      return "Transcribing...";
    case "Pasting":
      return "Inserting...";
    case "Ready":
      return "Done";
    case "Error":
      return appState.error?.message ?? "Check Scribe";
    default:
      return "";
  }
}

/**
 * Rolling waveform of recent mic levels. New samples enter on the right and
 * scroll left as the history shifts; a requestAnimationFrame loop eases the
 * displayed bars toward their targets by mutating `transform: scaleY(...)`
 * directly (no React state per frame, compositor-only updates).
 */
function Visualizer() {
  const barsRef = useRef<Array<HTMLSpanElement | null>>([]);
  const historyRef = useRef<number[]>(new Array(BAR_COUNT).fill(0));
  const displayRef = useRef<number[]>(new Array(BAR_COUNT).fill(0));

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    let frame = 0;

    void listen<AudioLevelEvent>("audio://level", (event) => {
      const history = historyRef.current;
      history.push(perceptualLevel(event.payload.rms));
      if (history.length > BAR_COUNT) {
        history.shift();
      }
    }).then((stop) => {
      unlisten = stop;
      if (disposed) {
        stop();
      }
    });

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
    frame = requestAnimationFrame(tick);

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      unlisten?.();
    };
  }, []);

  return (
    <div aria-hidden="true" className="pill-bars">
      {Array.from({ length: BAR_COUNT }, (_, i) => (
        <span
          className="pill-bar"
          key={i}
          ref={(el) => {
            barsRef.current[i] = el;
          }}
        />
      ))}
    </div>
  );
}

function PillApp() {
  const [appState, setAppState] = useState<AppStateSnapshot | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [stopping, setStopping] = useState(false);
  const [partial, setPartial] = useState<PartialTranscriptEvent | null>(null);
  const [confirmation, setConfirmation] = useState<{
    id: string;
    text: string;
  } | null>(null);
  const [copied, setCopied] = useState(false);
  const [noteSession, setNoteSession] = useState(false);
  const settingsRef = useRef<AppSettings | null>(null);
  const positionedRef = useRef(false);
  const sizedLayoutRef = useRef<PillLayout | null>(null);
  /** Logical height the window was last sized to (layout base or text growth). */
  const windowHeightRef = useRef<number | null>(null);
  const textRef = useRef<HTMLDivElement | null>(null);
  /** Serializes window size/position/show/hide mutations: concurrent ops read
   * stale positions and compound their bottom-anchoring shifts. */
  const windowOpChainRef = useRef<Promise<void>>(Promise.resolve());
  const pointerRef = useRef<{ x: number; y: number; dragging: boolean } | null>(
    null,
  );

  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const loadInitial = async () => {
      try {
        const [state, loadedSettings] = await Promise.all([
          getAppState(),
          getSettings(),
        ]);
        if (!disposed) {
          setAppState(state);
          setSettings(loadedSettings);
        }
      } catch {
        // Backend not ready yet; the window stays hidden until events arrive.
      }
    };

    const setup = async () => {
      const stop = await listen<AppStateSnapshot>(
        "scribe:app-state",
        (event) => {
          setAppState(event.payload);
          void getSettings()
            .then((latest) => {
              if (!disposed) {
                setSettings(latest);
              }
            })
            .catch(() => {
              // Keep the last known settings.
            });
        },
      );
      unlisten = stop;
      if (disposed) {
        stop();
      }
    };

    void loadInitial();
    void setup();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Track the live transcript for the active session, plus the final
  // transcript for the post-transcription confirmation state.
  useEffect(() => {
    let disposed = false;
    let unlisteners: Array<() => void> = [];

    const setup = async () => {
      unlisteners = await Promise.all([
        listen<PartialTranscriptEvent>(
          "scribe:partial-transcript",
          (event) => {
            // The payload carries the full accumulated text for its session,
            // so replacing wholesale also handles sessionId changes.
            setPartial(event.payload);
          },
        ),
        listen<DictationResult>("scribe:dictation-transcribed", (event) => {
          setConfirmation({
            id: event.payload.transcript.id,
            text: event.payload.transcript.text,
          });
          setCopied(false);
        }),
        listen<{ isNote: boolean }>("audio://recording-started", (event) => {
          setPartial(null);
          setConfirmation(null);
          setCopied(false);
          setNoteSession(event.payload.isNote === true);
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

  // The confirmation lingers for a few seconds, then the pill goes back to
  // following app state (which has usually gone Idle by then, hiding it).
  useEffect(() => {
    if (!confirmation) {
      return;
    }
    const timer = window.setTimeout(() => {
      setConfirmation(null);
      setCopied(false);
    }, CONFIRM_HIDE_DELAY_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [confirmation]);

  useEffect(() => {
    if (!copied) {
      return;
    }
    const timer = window.setTimeout(() => {
      setCopied(false);
    }, COPIED_RESET_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [copied]);

  // Persist the pill position after the user drags it.
  useEffect(() => {
    const pillWindow = getCurrentWindow();
    let disposed = false;
    let timer: number | null = null;
    let unlisten: (() => void) | null = null;

    void pillWindow
      .onMoved((event) => {
        if (timer !== null) {
          window.clearTimeout(timer);
        }
        const { x, y } = event.payload;
        timer = window.setTimeout(() => {
          void (async () => {
            const current = settingsRef.current;
            if (!current || (current.pillX === x && current.pillY === y)) {
              return;
            }
            // Never persist an off-screen position: Windows reports
            // (-32000, -32000) for a minimized/hidden window, which would
            // restore the pill where it can never be seen.
            if (!(await positionOnScreen(x, y))) {
              return;
            }
            const next = { ...current, pillX: x, pillY: y };
            settingsRef.current = next;
            setSettings(next);
            void updateSettings(next).catch(() => {
              // Position persistence is best-effort.
            });
          })();
        }, MOVE_PERSIST_DEBOUNCE_MS);
      })
      .then((stop) => {
        unlisten = stop;
        if (disposed) {
          stop();
        }
      });

    return () => {
      disposed = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
      unlisten?.();
    };
  }, []);

  const status = appState?.status ?? null;
  const showPill = settings?.showFloatingPill ?? false;
  const displayMode = settings?.pillDisplayMode ?? "visualizer_with_text";
  const pillColorNormal = settings?.pillColorNormal ?? "#fbbf24";
  const pillColorNote = settings?.pillColorNote ?? "#38bdf8";
  const pillBackgroundHex = settings?.pillColorBackground ?? "#0f1e38";
  const pillX = settings?.pillX ?? null;
  const pillY = settings?.pillY ?? null;
  const updatedAt = appState?.updatedAt ?? null;
  const isRecording = status === "Recording";
  const confirming = confirmation !== null;
  const layout = pillLayout(displayMode, status, isRecording, confirming);

  const queueWindowOp = useCallback((op: () => Promise<void>) => {
    windowOpChainRef.current = windowOpChainRef.current.then(op).catch(() => {
      // Window management is unavailable outside Tauri.
    });
  }, []);

  // Show/hide and resize the native window to match app state and layout.
  useEffect(() => {
    const pillWindow = getCurrentWindow();
    let hideTimer: number | null = null;

    const visible =
      showPill &&
      (confirming || (status !== null && VISIBLE_STATUSES.has(status)));

    if (!visible) {
      setPartial(null);
      queueWindowOp(() => pillWindow.hide());
      return;
    }

    const show = async () => {
      try {
        if (sizedLayoutRef.current !== layout) {
          const previousHeight = windowHeightRef.current;
          sizedLayoutRef.current = layout;
          const size = LAYOUT_SIZES[layout];
          windowHeightRef.current = size.height;
          // Keep the bottom edge anchored across layout changes so a taller
          // pill grows upward instead of clipping below the screen.
          const deltaY =
            previousHeight !== null && positionedRef.current
              ? Math.round(
                  (previousHeight - size.height) *
                    (await pillWindow.scaleFactor()),
                )
              : 0;
          const position = deltaY !== 0 ? await pillWindow.outerPosition() : null;
          await pillWindow.setSize(new LogicalSize(size.width, size.height));
          if (position && deltaY !== 0) {
            await pillWindow.setPosition(
              new PhysicalPosition(position.x, position.y + deltaY),
            );
          }
        }
        if (!positionedRef.current) {
          positionedRef.current = true;
          const hasSaved =
            typeof pillX === "number" && typeof pillY === "number";
          if (hasSaved && (await positionOnScreen(pillX!, pillY!))) {
            await pillWindow.setPosition(new PhysicalPosition(pillX!, pillY!));
          } else {
            // No saved position, or a stale one left off-screen (unplugged
            // monitor, Windows minimize sentinel): fall back to centering so
            // the pill self-heals instead of staying invisible.
            const monitor = await currentMonitor();
            if (monitor) {
              const size = await pillWindow.outerSize();
              const x = Math.round(
                monitor.position.x + (monitor.size.width - size.width) / 2,
              );
              const y = Math.round(
                monitor.position.y +
                  monitor.size.height -
                  size.height -
                  BOTTOM_MARGIN_PX,
              );
              await pillWindow.setPosition(new PhysicalPosition(x, y));
            }
          }
        }
        await pillWindow.show();
      } catch {
        // Window management is unavailable outside Tauri.
      }
    };

    queueWindowOp(show);

    if (status === "Ready" && !confirming) {
      hideTimer = window.setTimeout(() => {
        setPartial(null);
        queueWindowOp(() => pillWindow.hide());
      }, READY_HIDE_DELAY_MS);
    }

    return () => {
      if (hideTimer !== null) {
        window.clearTimeout(hideTimer);
      }
    };
  }, [status, showPill, pillX, pillY, updatedAt, confirming, layout, queueWindowOp]);

  // Grow the text-mode window with the transcript (and shrink back when a new
  // recording starts), keeping the bottom edge anchored. Measures how much
  // taller the clipped transcript wants to be and resizes between the compact
  // base height and TEXT_MAX_HEIGHT.
  const liveText = partial?.text ?? "";
  const confirmedText = confirmation?.text ?? "";
  useEffect(() => {
    if (layout !== "text" || !positionedRef.current) {
      return;
    }
    const pillWindow = getCurrentWindow();
    const grow = async () => {
      // Let the DOM settle so measurements reflect the latest text.
      await new Promise(requestAnimationFrame);
      const inner = textRef.current;
      const clipBox = inner?.parentElement;
      const current = windowHeightRef.current ?? LAYOUT_SIZES.text.height;
      if (!inner || !clipBox || sizedLayoutRef.current !== "text") {
        return;
      }
      // The clip box bottom-aligns its content, so text overflows off the
      // top edge and never shows up in scrollHeight; measure the inner
      // block's natural height against the box's visible height instead.
      const desired = Math.max(
        LAYOUT_SIZES.text.height,
        Math.min(
          TEXT_MAX_HEIGHT,
          current - clipBox.clientHeight + inner.offsetHeight,
        ),
      );
      if (Math.abs(desired - current) < 2) {
        return;
      }
      const deltaY = Math.round(
        (current - desired) * (await pillWindow.scaleFactor()),
      );
      const position = await pillWindow.outerPosition();
      windowHeightRef.current = desired;
      await pillWindow.setSize(
        new LogicalSize(LAYOUT_SIZES.text.width, desired),
      );
      if (deltaY !== 0) {
        await pillWindow.setPosition(
          new PhysicalPosition(position.x, position.y + deltaY),
        );
      }
    };
    queueWindowOp(grow);
  }, [layout, liveText, confirmedText, updatedAt, queueWindowOp]);

  const handleStop = useCallback(async () => {
    setStopping(true);

    try {
      const recording = await stopRecording();
      if (recording.status === "completed" || recording.status === "timed_out") {
        await transcribeRecording(recording);
      }
    } catch {
      // Errors surface in the main window via audio://recording-error.
    } finally {
      setStopping(false);
    }
  }, []);

  const handleCopy = useCallback(async () => {
    const current = confirmation;
    if (!current) {
      return;
    }
    try {
      await copyTranscript(current.id);
    } catch {
      try {
        await navigator.clipboard.writeText(current.text);
      } catch {
        return;
      }
    }
    setCopied(true);
  }, [confirmation]);

  // Manual click-vs-drag discrimination: data-tauri-drag-region would swallow
  // clicks, so we start the OS drag ourselves once the pointer travels past a
  // small threshold, and treat a still pointerup as a click (stop recording).
  const handlePointerDown = useCallback((event: React.PointerEvent) => {
    if (event.button !== 0) {
      return;
    }
    pointerRef.current = { x: event.clientX, y: event.clientY, dragging: false };
  }, []);

  const handlePointerMove = useCallback((event: React.PointerEvent) => {
    const start = pointerRef.current;
    if (!start || start.dragging) {
      return;
    }
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    if (dx * dx + dy * dy >= DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX) {
      start.dragging = true;
      // Hands the gesture to the OS; no further pointer events arrive, and
      // onMoved persistence picks the new position up afterwards.
      void getCurrentWindow().startDragging().catch(() => {});
    }
  }, []);

  const handlePointerUp = useCallback(() => {
    const start = pointerRef.current;
    pointerRef.current = null;
    if (start && !start.dragging && isRecording && !stopping) {
      void handleStop();
    }
  }, [handleStop, isRecording, stopping]);

  if (!appState) {
    return null;
  }

  const partialText = partial?.text.trim() ?? "";
  const label = pillLabel(appState);

  const noteTone =
    noteSession &&
    !confirming &&
    (isRecording ||
      status === "Stopping" ||
      status === "Transcribing" ||
      status === "Pasting");

  return (
    <div
      className={`pill-shell ${pillTone(appState.status)} layout-${layout}${noteTone ? " note-session" : ""}`}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      style={
        {
          "--pill-bar": pillColorNormal,
          "--pill-bar-note": pillColorNote,
          "--pill-bg": hexWithAlpha(pillBackgroundHex, 0.84),
          "--pill-bg-hover": hexWithAlpha(pillBackgroundHex, 0.94),
        } as React.CSSProperties
      }
      title={
        isRecording
          ? "Click to stop - drag to move"
          : layout === "dot" && label
            ? label
            : undefined
      }
    >
      {confirmation ? (
        <div className="pill-confirm">
          <div className="pill-confirm-head">
            <Check aria-hidden="true" className="pill-check" size={14} />
            <span className="pill-confirm-label">Transcribed</span>
            <button
              className={`pill-copy${copied ? " copied" : ""}`}
              onClick={() => void handleCopy()}
              onPointerDown={(event) => event.stopPropagation()}
              type="button"
            >
              {copied ? "Copied" : "Copy"}
            </button>
          </div>
          {layout === "text" ? (
            <div className="pill-text">
              <div ref={textRef}>{confirmation.text}</div>
            </div>
          ) : null}
        </div>
      ) : isRecording ? (
        displayMode === "dot" ? (
          <span aria-hidden="true" className="pill-pulse" />
        ) : (
          <div className="pill-rec">
            <Visualizer />
            {displayMode === "visualizer_with_text" ? (
              <div className="pill-text">
                <div ref={textRef}>{partialText || "Listening..."}</div>
              </div>
            ) : null}
          </div>
        )
      ) : (
        <>
          {appState.status === "Ready" ? (
            <Check aria-hidden="true" className="pill-check" size={14} />
          ) : (
            <span aria-hidden="true" className="pill-pulse" />
          )}
          {layout === "dot" ? null : (
            <span className="pill-label" title={label}>
              {label}
            </span>
          )}
        </>
      )}
    </div>
  );
}

export default PillApp;
