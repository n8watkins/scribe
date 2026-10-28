import type { AppSettings } from "../backend";

/**
 * The fields of `patch` whose values differ from `current`, ready to send as a
 * settings patch. A window only sends what it actually changed, so it never
 * writes back a field another window updated in the meantime. `undefined`
 * becomes `null` (JSON would drop the key, and the backend reads `null` as
 * "unset").
 */
export function changedSettings(
  current: AppSettings,
  patch: Partial<AppSettings>,
): Partial<AppSettings> {
  const changes: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(patch)) {
    const next = value === undefined ? null : value;
    const previous = (current as Record<string, unknown>)[key] ?? null;
    if (JSON.stringify(next) !== JSON.stringify(previous)) {
      changes[key] = next;
    }
  }
  return changes as Partial<AppSettings>;
}

/** The values `settings` has for the keys of `patch`, to undo an optimistic
 * update of just those keys. */
export function settingsFor(
  settings: AppSettings,
  patch: Partial<AppSettings>,
): Partial<AppSettings> {
  const values: Record<string, unknown> = {};
  for (const key of Object.keys(patch)) {
    values[key] = (settings as Record<string, unknown>)[key];
  }
  return values as Partial<AppSettings>;
}
