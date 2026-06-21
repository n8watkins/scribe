/**
 * Derivation of the full `--scribe-*` palette from the three core colors of the
 * user-defined "custom" theme (background, accent, text).
 *
 * The preset dark themes in App.css relate their tones in a consistent way — the
 * elevated surface is a touch lighter than the page, inset surfaces step up from
 * there, borders are a faint tint of the text color, the accent family is the
 * accent plus lighter variants, and the secondary/muted/faint text tones are the
 * text color progressively mixed toward the background. These helpers reproduce
 * that relationship so a custom dark theme stays coherent across the whole UI.
 *
 * Everything here is pure (no DOM access), so App.tsx can derive the variables
 * and apply them, and the result is easy to reason about in isolation.
 */

export type CustomThemeColors = {
  background: string;
  accent: string;
  text: string;
};

type Rgb = { r: number; g: number; b: number };

const clamp = (n: number): number => Math.max(0, Math.min(255, Math.round(n)));

/** Parse a `#rrggbb` (or `#rgb`) hex string to channels. Falls back to black on
 *  anything unparseable, so a half-typed color in the picker never throws. */
function parseHex(hex: string): Rgb {
  const h = hex.trim().replace(/^#/, "");
  const full =
    h.length === 3
      ? h
          .split("")
          .map((c) => c + c)
          .join("")
      : h;
  if (!/^[0-9a-fA-F]{6}$/.test(full)) {
    return { r: 0, g: 0, b: 0 };
  }
  return {
    r: parseInt(full.slice(0, 2), 16),
    g: parseInt(full.slice(2, 4), 16),
    b: parseInt(full.slice(4, 6), 16),
  };
}

function toHex({ r, g, b }: Rgb): string {
  const part = (n: number) => clamp(n).toString(16).padStart(2, "0");
  return `#${part(r)}${part(g)}${part(b)}`;
}

/** Move each channel `amt` (0–1) of the way toward white. */
export function lighten(hex: string, amt: number): string {
  const { r, g, b } = parseHex(hex);
  return toHex({
    r: r + (255 - r) * amt,
    g: g + (255 - g) * amt,
    b: b + (255 - b) * amt,
  });
}

/** Move each channel `amt` (0–1) of the way toward black. */
export function darken(hex: string, amt: number): string {
  const { r, g, b } = parseHex(hex);
  return toHex({
    r: r * (1 - amt),
    g: g * (1 - amt),
    b: b * (1 - amt),
  });
}

/** Linear blend of two colors: t=0 → a, t=1 → b. */
export function mix(a: string, b: string, t: number): string {
  const ca = parseHex(a);
  const cb = parseHex(b);
  return toHex({
    r: ca.r + (cb.r - ca.r) * t,
    g: ca.g + (cb.g - ca.g) * t,
    b: ca.b + (cb.b - ca.b) * t,
  });
}

/** A `rgba(r, g, b, a)` string from a hex color and an alpha (0–1). */
export function rgba(hex: string, alpha: number): string {
  const { r, g, b } = parseHex(hex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** WCAG relative luminance (0 = black, 1 = white). */
export function relativeLuminance(hex: string): number {
  const { r, g, b } = parseHex(hex);
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** The raw "r, g, b" channel triple of a hex color (for `--scribe-surface-rgb`). */
function rgbTriple(hex: string): string {
  const { r, g, b } = parseHex(hex);
  return `${r}, ${g}, ${b}`;
}

/**
 * Derive the complete `--scribe-*` palette for the custom theme. Returns a plain
 * record so the caller can `style.setProperty(name, value)` for each entry (and
 * `removeProperty` the same keys when switching back to a preset).
 */
export function deriveCustomThemeVars(colors: CustomThemeColors): Record<string, string> {
  const { background, accent, text } = colors;
  // Light accents need dark text on top; dark accents need light text. The 0.5
  // luminance split mirrors how the presets pick `--scribe-accent-on`.
  const accentOn =
    relativeLuminance(accent) > 0.5 ? darken(background, 0.05) : "#f8fafc";

  return {
    "--scribe-bg": background,
    "--scribe-bg-elevated": lighten(background, 0.06),
    "--scribe-sidebar-bg": rgba(lighten(background, 0.04), 0.9),
    "--scribe-card-gradient-top": rgba(lighten(background, 0.12), 0.82),
    "--scribe-card-gradient-bottom": rgba(background, 0.92),
    "--scribe-border": rgba(text, 0.18),
    "--scribe-glow-1": rgba(accent, 0.12),
    "--scribe-glow-2": rgba(accent, 0.08),
    "--scribe-surface-rgb": rgbTriple(background),
    "--scribe-surface-solid": lighten(background, 0.08),
    "--scribe-surface-strong": lighten(background, 0.14),
    "--scribe-surface-sunken": darken(background, 0.02),
    "--scribe-surface-raised": lighten(background, 0.04),
    "--scribe-surface-kbd": darken(background, 0.02),
    "--scribe-accent": accent,
    "--scribe-accent-strong": lighten(accent, 0.1),
    "--scribe-accent-bright": lighten(accent, 0.2),
    "--scribe-accent-on": accentOn,
    "--scribe-accent-soft": rgba(accent, 0.26),
    "--scribe-accent-glow": rgba(accent, 0.1),
    "--scribe-accent-ring": rgba(accent, 0.14),
    "--scribe-scrollbar-thumb": rgba(accent, 0.3),
    "--scribe-scrollbar-thumb-hover": rgba(accent, 0.55),
    "--scribe-text": text,
    "--scribe-text-secondary": mix(text, background, 0.18),
    "--scribe-text-muted": mix(text, background, 0.42),
    "--scribe-text-faint": mix(text, background, 0.58),
  };
}
