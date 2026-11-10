// Where each speech model sits on accuracy and speed (1-5 bars), so the
// order Tiny < Base < Small < Medium < Large is obvious, plus what each
// engine supports. From a benchmark on dictation with developer jargon
// (whisper.cpp v1.9.1 on a CPU, November 2026).

export type ModelGuide = {
  /** 1 (misses the most words) to 5 (gets the most right). */
  accuracy: number;
  /** 1 (slowest) to 5 (fastest) on a CPU. */
  speed: number;
  recommended?: boolean;
};

const BY_SIZE: Record<string, ModelGuide> = {
  tiny: { accuracy: 1, speed: 5 },
  base: { accuracy: 2, speed: 4 },
  small: { accuracy: 3, speed: 3 },
  medium: { accuracy: 4, speed: 2 },
  large: { accuracy: 5, speed: 1 },
};

/** The guide for a catalog model, or null for an unknown one. */
export function modelGuide(id: string, engine: string): ModelGuide | null {
  if (engine === "parakeet") {
    return { accuracy: 3, speed: 5 };
  }
  const size = /^(tiny|base|small|medium|large)/.exec(id)?.[1];
  if (!size) {
    return null;
  }
  return id === "small.en-q5_1"
    ? { ...BY_SIZE[size], recommended: true }
    : BY_SIZE[size];
}

/** What an engine does with your settings, as a short checklist. */
export function engineSupport(
  engine: string,
): { label: string; supported: boolean }[] {
  return [
    { label: "Custom words", supported: engine !== "parakeet" },
    { label: "Replacements", supported: true },
    { label: "Filler removal", supported: true },
  ];
}
