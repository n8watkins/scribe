import { Check, Palette } from "lucide-react";
import type { AppSettings } from "../backend";
import type { ViewActions } from "../types";

/** A selectable color theme: its stored key, the label shown on its card, a
 * one-line description, and a few representative swatch colors. The swatches are
 * literal copies of the palette defined for `[data-theme="<key>"]` in App.css,
 * so each card previews how the theme recolors the app. */
type ThemeOption = {
  key: string;
  name: string;
  description: string;
  /** Page background, accent, and a surface/text tone — enough to read the mood. */
  swatches: string[];
};

const THEMES: ThemeOption[] = [
  {
    key: "midnight",
    name: "Midnight",
    description: "The classic deep navy with a cyan accent.",
    swatches: ["#070b14", "#0d1320", "#22d3ee", "#38bdf8", "#f8fafc"],
  },
  {
    key: "ocean",
    name: "Ocean",
    description: "Teal-leaning depths with a bright sky-blue accent.",
    swatches: ["#04141c", "#0a2230", "#2dd4bf", "#38bdf8", "#ecfeff"],
  },
  {
    key: "slate",
    name: "Slate",
    description: "Neutral graphite with a soft indigo accent.",
    swatches: ["#0b0e14", "#151a23", "#818cf8", "#a5b4fc", "#f1f5f9"],
  },
  {
    key: "emerald",
    name: "Emerald",
    description: "Dark forest ground with a vivid green accent.",
    swatches: ["#05140d", "#0a2117", "#10b981", "#34d399", "#ecfdf5"],
  },
  {
    key: "violet",
    name: "Violet",
    description: "Deep plum with a luminous purple accent.",
    swatches: ["#0c0a1a", "#181233", "#a78bfa", "#c084fc", "#f5f3ff"],
  },
  {
    key: "daylight",
    name: "Daylight",
    description: "A clean light theme: soft off-white surfaces and a deep-blue accent.",
    swatches: ["#e2e8f0", "#f9fbfd", "#0369a1", "#075985", "#0f172a"],
  },
];

/** The custom theme is rendered separately from THEMES (it has no fixed swatch
 * list — its preview is built live from the user's five core colors). */
const CUSTOM_KEY = "custom";

export function ThemesView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  // Fall back to "midnight" so an unknown/blank stored value still highlights a
  // card (matches the backend default).
  const activeTheme = settings.theme || "midnight";
  const isCustomActive = activeTheme === CUSTOM_KEY;
  const custom = settings.customTheme;
  // Live preview chips for the Custom tile, derived from the user's core colors.
  // Five swatches so the tile reads the same way as the preset tiles.
  const customSwatches = [
    custom.background,
    custom.surface,
    custom.accent,
    custom.text,
    custom.textMuted,
  ];

  const setCustomColor = (
    key: "background" | "surface" | "accent" | "text" | "textMuted",
    value: string,
  ) => actions.updateSettings({ customTheme: { ...custom, [key]: value } });

  return (
    <section className="stack">
      <article className="panel-card">
        <div className="section-heading compact">
          <h2>
            <Palette aria-hidden="true" size={15} />
            Color theme
          </h2>
        </div>
        <p className="muted" style={{ margin: "0 0 4px" }}>
          Choose a color theme for the main window. The floating pill keeps its
          own colors (set those in Audio &rarr; Pill).
        </p>

        <div className="theme-grid" role="radiogroup" aria-label="Color theme">
          {THEMES.map((theme) => {
            const isActive = theme.key === activeTheme;
            return (
              <button
                aria-checked={isActive}
                aria-label={theme.name}
                className={isActive ? "theme-card is-active" : "theme-card"}
                disabled={actions.savingSettings && !isActive}
                key={theme.key}
                onClick={() => actions.updateSettings({ theme: theme.key })}
                role="radio"
                type="button"
              >
                <div className="theme-swatches" aria-hidden="true">
                  {theme.swatches.map((color, index) => (
                    <span
                      className="theme-chip"
                      key={index}
                      style={{ background: color }}
                    />
                  ))}
                </div>
                <div className="theme-card-body">
                  <strong>{theme.name}</strong>
                  <small>{theme.description}</small>
                </div>
                {isActive ? (
                  <span className="theme-card-check" aria-hidden="true">
                    <Check size={13} />
                  </span>
                ) : null}
              </button>
            );
          })}

          <button
            aria-checked={isCustomActive}
            aria-label="Custom"
            className={isCustomActive ? "theme-card is-active" : "theme-card"}
            disabled={actions.savingSettings && !isCustomActive}
            onClick={() => actions.updateSettings({ theme: CUSTOM_KEY })}
            role="radio"
            type="button"
          >
            <div className="theme-swatches" aria-hidden="true">
              {customSwatches.map((color, index) => (
                <span
                  className="theme-chip"
                  key={index}
                  style={{ background: color }}
                />
              ))}
            </div>
            <div className="theme-card-body">
              <strong>Custom</strong>
              <small>Your own palette from five core colors.</small>
            </div>
            {isCustomActive ? (
              <span className="theme-card-check" aria-hidden="true">
                <Check size={13} />
              </span>
            ) : null}
          </button>
        </div>

        {isCustomActive ? (
          <div className="custom-theme-controls">
            <p className="muted" style={{ margin: "8px 0 4px" }}>
              Pick a background, surface, accent, text, and muted-text color. The
              rest of the palette (borders, secondary text, glows) is derived to
              match.
            </p>
            <div className="custom-theme-row">
              <CustomColorField
                disabled={actions.savingSettings}
                label="Background"
                onChange={(value) => setCustomColor("background", value)}
                value={custom.background}
              />
              <CustomColorField
                disabled={actions.savingSettings}
                label="Surface"
                onChange={(value) => setCustomColor("surface", value)}
                value={custom.surface}
              />
              <CustomColorField
                disabled={actions.savingSettings}
                label="Accent"
                onChange={(value) => setCustomColor("accent", value)}
                value={custom.accent}
              />
              <CustomColorField
                disabled={actions.savingSettings}
                label="Text"
                onChange={(value) => setCustomColor("text", value)}
                value={custom.text}
              />
              <CustomColorField
                disabled={actions.savingSettings}
                label="Muted text"
                onChange={(value) => setCustomColor("textMuted", value)}
                value={custom.textMuted}
              />
            </div>
          </div>
        ) : null}
      </article>
    </section>
  );
}

/** One labelled `<input type="color">` for the custom theme, with the current
 * hex shown beside the swatch. */
function CustomColorField({
  disabled,
  label,
  onChange,
  value,
}: {
  disabled: boolean;
  label: string;
  onChange: (value: string) => void;
  value: string;
}) {
  return (
    <label className="custom-theme-field">
      <span className="custom-theme-field-label">{label}</span>
      <span className="custom-theme-field-input">
        <input
          aria-label={`${label} color`}
          disabled={disabled}
          onChange={(event) => onChange(event.currentTarget.value)}
          type="color"
          value={value}
        />
        <code>{value}</code>
      </span>
    </label>
  );
}
