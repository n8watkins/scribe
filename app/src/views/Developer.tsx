import { useEffect, useState } from "react";
import { getName as getAppName } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Gauge, Keyboard } from "lucide-react";
import {
  commandErrorMessage,
  loadProductionHotkeyDefaults,
  type AppSettings,
} from "../backend";
import type { ViewActions } from "../types";
import { SectionPanel, SettingRow } from "../components/layout";

export function DeveloperView({
  refresh,
}: {
  actions: ViewActions;
  refresh: () => Promise<void>;
  settings: AppSettings;
}) {
  // The webview content size is what CSS breakpoints (e.g. .status-grid
  // auto-fit) actually respond to, so it's the must-have readout.
  const [contentSize, setContentSize] = useState({
    width: window.innerWidth,
    height: window.innerHeight,
  });
  // The Tauri outer window size (physical pixels incl. frame). Nice-to-have;
  // unavailable outside the Tauri runtime, so failures are ignored.
  const [outerSize, setOuterSize] = useState<{
    width: number;
    height: number;
  } | null>(null);
  // The "Load production defaults" control only matters for Scribe Dev (which
  // seeds its own non-conflicting binds).
  const [isDevFlavor, setIsDevFlavor] = useState(false);
  const [loadingDefaults, setLoadingDefaults] = useState(false);
  const [defaultsNotice, setDefaultsNotice] = useState<string | null>(null);

  useEffect(() => {
    void getAppName()
      .then((name) => setIsDevFlavor(name.includes("Dev")))
      .catch(() => {});
  }, []);

  const handleLoadProductionDefaults = async () => {
    setLoadingDefaults(true);
    setDefaultsNotice(null);
    try {
      await loadProductionHotkeyDefaults();
      await refresh();
      setDefaultsNotice(
        "Loaded your production binds. Quit stable Scribe to avoid conflicts.",
      );
    } catch (cause) {
      setDefaultsNotice(commandErrorMessage(cause));
    } finally {
      setLoadingDefaults(false);
    }
  };

  useEffect(() => {
    const onResize = () =>
      setContentSize({
        width: window.innerWidth,
        height: window.innerHeight,
      });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    const readOuter = async () => {
      try {
        const size = await getCurrentWindow().outerSize();
        if (!disposed) {
          setOuterSize({ width: size.width, height: size.height });
        }
      } catch {
        // Not running under Tauri (or the call failed); content size stands in.
      }
    };

    void readOuter();
    // Keep the outer size fresh as the window is resized.
    void getCurrentWindow()
      .onResized(() => {
        void readOuter();
      })
      .then((dispose) => {
        if (disposed) {
          dispose();
        } else {
          unlisten = dispose;
        }
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return (
    <section className="view-grid">
      <SectionPanel
        icon={<Gauge aria-hidden="true" size={16} />}
        title="Window resolution"
      >
        <SettingRow
          description="The webview content size (window.innerWidth x innerHeight). This is the value CSS breakpoints respond to."
          label="Content size (CSS px)"
        >
          <strong>
            {contentSize.width} x {contentSize.height}
          </strong>
        </SettingRow>
        <SettingRow
          description="The Tauri outer window size in physical pixels, including the window frame."
          label="Outer window (physical px)"
        >
          <strong>
            {outerSize ? `${outerSize.width} x ${outerSize.height}` : "Unavailable"}
          </strong>
        </SettingRow>
        <p className="muted vocab-hint">
          Resize the window to watch these update live. More developer
          diagnostics will land in this panel.
        </p>
      </SectionPanel>

      {isDevFlavor ? (
        <SectionPanel
          icon={<Keyboard aria-hidden="true" size={16} />}
          title="Developer hotkeys"
        >
          <SettingRow
            description="Scribe Dev seeds non-conflicting binds (Ctrl+Shift+ variants) so it can run alongside stable Scribe. Load your production binds to use your real shortcuts when running Dev alone."
            label="Load my production defaults"
          >
            <button
              className="secondary-button"
              disabled={loadingDefaults}
              onClick={() => void handleLoadProductionDefaults()}
              type="button"
            >
              <Keyboard aria-hidden="true" size={15} />
              {loadingDefaults ? "Loading..." : "Load production defaults"}
            </button>
          </SettingRow>
          {defaultsNotice ? (
            <p className="muted vocab-hint">{defaultsNotice}</p>
          ) : null}
        </SectionPanel>
      ) : null}
    </section>
  );
}
