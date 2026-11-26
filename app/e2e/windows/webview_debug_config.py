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
    python webview_debug_config.py --conf app/src-tauri/tauri.conf.json --cargo-lock Cargo.lock --port 9222 --out overlay.json
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

# What wry passes when a window sets no arguments (wry's webview2 module, with
# Tauri's default autoplay); kept so only the port differs from a normal build.
# Copied from this wry version: a different one in Cargo.lock fails the build
# until the arguments are checked against its src/webview2/mod.rs again.
WRY_DEFAULT_ARGS = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required"
WRY_DEFAULT_ARGS_VERSION = "0.57.1"


def locked_versions(lock: Path, name: str) -> list[str]:
    """Every version of package `name` in a Cargo.lock."""
    versions = []
    for block in lock.read_text(encoding="utf-8").split("[[package]]"):
        fields = dict(re.findall(r'^(\w+) = "([^"]*)"$', block, re.MULTILINE))
        if fields.get("name") == name:
            versions.append(fields["version"])
    return versions


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--conf", required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--out", required=True)
    parser.add_argument("--cargo-lock", required=True, help="the workspace Cargo.lock, for wry's version")
    args = parser.parse_args()
    wry = locked_versions(Path(args.cargo_lock), "wry")
    if wry != [WRY_DEFAULT_ARGS_VERSION]:
        raise SystemExit(
            f"Cargo.lock has wry {wry}, but WRY_DEFAULT_ARGS were copied from wry {WRY_DEFAULT_ARGS_VERSION}. "
            "Compare them with that wry's src/webview2/mod.rs (additional_browser_args) and update both."
        )
    windows = json.loads(Path(args.conf).read_text(encoding="utf-8"))["app"]["windows"]
    for window in windows:
        if "additionalBrowserArgs" in window:
            raise SystemExit(f"Window {window.get('label')} already sets additionalBrowserArgs; merge them here")
        window["additionalBrowserArgs"] = f"{WRY_DEFAULT_ARGS} --remote-debugging-port={args.port}"
    Path(args.out).write_text(json.dumps({"app": {"windows": windows}}, indent=2), encoding="utf-8")
    print(f"WebView2 debugging port {args.port} for windows {[w.get('label') for w in windows]}")


if __name__ == "__main__":
    main()
