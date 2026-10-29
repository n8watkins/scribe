import { afterEach, describe, expect, it } from "vitest";
import { commandErrorMessage } from "../backend";
import { currentPlatform, forPlatform } from "./platform";

afterEach(() => {
  delete window.__SCRIBE_PLATFORM__;
});

describe("currentPlatform", () => {
  it("reads the platform the backend injects", () => {
    window.__SCRIBE_PLATFORM__ = "linux";
    expect(currentPlatform()).toBe("linux");
    window.__SCRIBE_PLATFORM__ = "macos";
    expect(currentPlatform()).toBe("macos");
  });

  it("falls back to Windows when nothing or something unknown is injected", () => {
    expect(currentPlatform()).toBe("windows");
    window.__SCRIBE_PLATFORM__ = "freebsd";
    expect(currentPlatform()).toBe("windows");
  });

  it("picks the variant for the current platform", () => {
    const variants = { windows: "w", macos: "m", linux: "l" };
    expect(forPlatform(variants)).toBe("w");
    window.__SCRIBE_PLATFORM__ = "linux";
    expect(forPlatform(variants)).toBe("l");
  });
});

describe("platform-specific error copy", () => {
  const message = (code: string) => commandErrorMessage({ code, message: "raw" });

  it("keeps the Windows wording on Windows", () => {
    expect(message("autostart_failed")).toContain("Windows rejected the startup registration");
    expect(message("missing_whisper_executable")).toContain(
      "resources/bin/windows/whisper-cli.exe",
    );
  });

  it("never mentions Windows on Linux or macOS", () => {
    const codes = [
      "microphone_permission_denied",
      "missing_whisper_executable",
      "autostart_failed",
      "app_data_dir_unavailable",
    ];
    for (const platform of ["linux", "macos"]) {
      window.__SCRIBE_PLATFORM__ = platform;
      for (const code of codes) {
        expect(message(code)).not.toMatch(/windows/i);
      }
    }
  });

  it("points at this platform's whisper binary", () => {
    window.__SCRIBE_PLATFORM__ = "linux";
    expect(message("missing_whisper_executable")).toContain("resources/bin/linux/whisper-cli.");
    window.__SCRIBE_PLATFORM__ = "macos";
    expect(message("missing_whisper_executable")).toContain("resources/bin/macos/whisper-cli.");
  });
});
