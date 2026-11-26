"""Writes a Tauri config overlay that opens WebView2's remote debugging port
in the end-to-end test build, so run.py can call the app's own commands (the
ones the Hotkeys page calls) inside the dashboard.

WebView2 takes its browser arguments from the app when the app passes any,
and wry always does, so neither WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS nor the
loader's policy key reaches it at run time. The overlay sets
`additionalBrowserArgs` on every window in tauri.conf.json (Tauri's --config
merge replaces arrays, so each window is copied whole) with wry's default
arguments plus the port. Every window must get the same arguments: they share
one WebView2 browser process.

Usage:
    python webview_debug_config.py --conf app/src-tauri/tauri.conf.json --port 9222 --out overlay.json
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

# What wry passes when a window sets no arguments (wry's webview2 module, with
# Tauri's default autoplay); kept so only the port differs from a normal build.
WRY_DEFAULT_ARGS = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--conf", required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    windows = json.loads(Path(args.conf).read_text(encoding="utf-8"))["app"]["windows"]
    for window in windows:
        if "additionalBrowserArgs" in window:
            raise SystemExit(f"Window {window.get('label')} already sets additionalBrowserArgs; merge them here")
        window["additionalBrowserArgs"] = f"{WRY_DEFAULT_ARGS} --remote-debugging-port={args.port}"
    Path(args.out).write_text(json.dumps({"app": {"windows": windows}}, indent=2), encoding="utf-8")
    print(f"WebView2 debugging port {args.port} for windows {[w.get('label') for w in windows]}")


if __name__ == "__main__":
    main()
