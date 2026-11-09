import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  getVersion: vi.fn(),
  checkForUpdate: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: mocks.getVersion }));
vi.mock("../backend", () => ({ checkForUpdate: mocks.checkForUpdate }));

import { detectUpdate, updatesInstallInApp } from "./updates";

beforeEach(() => {
  mocks.check.mockReset();
  mocks.getVersion.mockReset().mockResolvedValue("0.11.0");
  mocks.checkForUpdate.mockReset();
});

afterEach(() => {
  delete window.__SCRIBE_PLATFORM__;
});

describe("detectUpdate", () => {
  it("asks the signed updater on Windows", async () => {
    mocks.check.mockResolvedValue({ version: "0.12.0", body: " Notes " });
    expect(updatesInstallInApp()).toBe(true);
    await expect(detectUpdate()).resolves.toMatchObject({
      updateAvailable: true,
      currentVersion: "0.11.0",
      latestVersion: "0.12.0",
      releaseNotes: "Notes",
    });
    expect(mocks.checkForUpdate).not.toHaveBeenCalled();
  });

  it.each(["macos", "linux"])(
    "on %s reads the version from the backend, where the updater has no package",
    async (platform) => {
      window.__SCRIBE_PLATFORM__ = platform;
      const result = {
        updateAvailable: true,
        currentVersion: "0.11.0",
        latestVersion: "0.12.0",
        releaseUrl: "https://github.com/n8watkins/scribe/releases/latest",
      };
      mocks.checkForUpdate.mockResolvedValue(result);
      expect(updatesInstallInApp()).toBe(false);
      await expect(detectUpdate()).resolves.toEqual(result);
      expect(mocks.check).not.toHaveBeenCalled();
    },
  );
});
