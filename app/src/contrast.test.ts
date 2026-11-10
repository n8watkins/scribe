import { describe, expect, it } from "vitest";
import app from "./App.css?raw";

// Every text color must reach WCAG AA (4.5:1) on every surface it sits on, in
// every theme. Surfaces: the page, the panel base and the lighter top of the
// card gradient laid over it.

type Rgba = [number, number, number, number];

function parse(color: string): Rgba {
  const value = color.trim();
  if (value.startsWith("#")) {
    const hex = value.slice(1);
    const full = hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex;
    return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16)).concat(1) as Rgba;
  }
  const parts = /rgba?\(([^)]+)\)/.exec(value)?.[1].split(",").map(Number);
  if (!parts) {
    throw new Error(`Unreadable color ${value}`);
  }
  return [parts[0], parts[1], parts[2], parts[3] ?? 1];
}

const over = (top: Rgba, bottom: Rgba): Rgba =>
  [0, 1, 2].map((i) => top[i] * top[3] + bottom[i] * (1 - top[3])).concat(1) as Rgba;

function luminance([r, g, b]: Rgba): number {
  const channel = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: Rgba, b: Rgba): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** Each theme's tokens, with the :root defaults underneath. */
function themes(): Record<string, Record<string, string>> {
  const blocks: Record<string, Record<string, string>> = {};
  for (const match of app.matchAll(/:root(?:\[data-theme="([a-z]+)"\])?\s*\{([^}]*)\}/g)) {
    const name = match[1] ?? "root";
    const vars = (blocks[name] ??= {});
    for (const v of match[2].matchAll(/(--scribe-[a-z0-9-]+):\s*([^;]+);/g)) {
      vars[v[1]] = v[2].trim();
    }
  }
  const { root, ...named } = blocks;
  return Object.fromEntries(
    Object.entries(named).map(([name, vars]) => [name, { ...root, ...vars }]),
  );
}

const TEXT = [
  "--scribe-text",
  "--scribe-text-secondary",
  "--scribe-text-muted",
  "--scribe-text-faint",
];

describe("text contrast", () => {
  const all = themes();

  it("finds every preset theme", () => {
    expect(Object.keys(all).sort()).toEqual(
      ["daylight", "emerald", "midnight", "ocean", "slate", "violet"].sort(),
    );
  });

  for (const [theme, vars] of Object.entries(all)) {
    it(`${theme}: every text color reaches 4.5:1 on every surface`, () => {
      const elevated = parse(vars["--scribe-bg-elevated"]);
      const surfaces = {
        page: parse(vars["--scribe-bg"]),
        panel: elevated,
        cardTop: over(parse(vars["--scribe-card-gradient-top"]), elevated),
      };
      for (const token of TEXT) {
        for (const [surface, color] of Object.entries(surfaces)) {
          const ratio = contrast(parse(vars[token]), color);
          expect(ratio, `${token} on ${surface}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    });
  }
});
