export type Platform = "windows" | "macos" | "linux";

declare global {
  interface Window {
    /** Set by the backend's init script before any page script runs. */
    __SCRIBE_PLATFORM__?: string;
  }
}

/**
 * The OS Scribe runs on. The backend injects it into every webview before the
 * page loads, so it is known synchronously and copy never flashes the wrong
 * wording. Without it (unit tests, a plain browser preview) this is Windows,
 * the platform Scribe ships for.
 */
export function currentPlatform(): Platform {
  const value = typeof window === "undefined" ? undefined : window.__SCRIBE_PLATFORM__;
  return value === "macos" || value === "linux" ? value : "windows";
}

/** Picks the variant of a piece of copy for the current platform. */
export function forPlatform<T>(variants: Record<Platform, T>): T {
  return variants[currentPlatform()];
}
