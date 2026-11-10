// Plain-words guide to each speech model: where it sits on size, accuracy
// and speed, so the order Tiny < Base < Small < Medium < Large is obvious.
// Accuracy and speed are 1-5, from a benchmark on dictation with developer
// jargon (whisper.cpp v1.9.1 on a CPU, November 2026).

export type ModelGuide = {
  /** 1 (misses the most words) to 5 (gets the most right). */
  accuracy: number;
  /** 1 (slowest) to 5 (fastest) on a CPU. */
  speed: number;
  /** One plain line under the model's name. */
  note: string;
  recommended?: boolean;
};

const BY_SIZE: Record<string, ModelGuide> = {
  tiny: {
    accuracy: 1,
    speed: 5,
    note: "Smallest and fastest. Misses the most words.",
  },
  base: {
    accuracy: 2,
    speed: 4,
    note: "Small and quick. A fine start on a slow computer.",
  },
  small: {
    accuracy: 3,
    speed: 3,
    note: "Gets clearly more words right than Base, in about twice the time.",
  },
  medium: {
    accuracy: 4,
    speed: 2,
    note: "More accurate again, but slow without a graphics card.",
  },
  large: {
    accuracy: 5,
    speed: 1,
    note: "The most accurate. Too slow for live dictation without a graphics card.",
  },
};

export const PARAKEET_GUIDE: ModelGuide = {
  accuracy: 3,
  speed: 5,
  note: "The fastest for live dictation, with punctuation built in.",
};

/** The guide for a catalog model id, or null for an unknown model. */
export function modelGuide(id: string, engine: string): ModelGuide | null {
  if (engine === "parakeet") {
    return PARAKEET_GUIDE;
  }
  const size = /^(tiny|base|small|medium|large)/.exec(id)?.[1];
  if (!size) {
    return null;
  }
  const guide = BY_SIZE[size];
  if (id === "small.en-q5_1") {
    return {
      ...guide,
      note: "Gets clearly more words right than Base, in about twice the time. A compressed copy of Small: a third of the download.",
      recommended: true,
    };
  }
  return guide;
}

/** What Parakeet gives up and gains, shown before you pick it. Times are a
 * 5 s dictation on 4 CPU threads in the benchmark. */
export const PARAKEET_TRADE = {
  gains:
    "Very fast: about 0.5 s for a 5 s dictation, where Base takes about 1.8 s. Its time grows with what you say, while Whisper always works through 30 s.",
  givesUp:
    "Doesn't use your custom words. Replacements still work, and so does filler removal.",
} as const;
