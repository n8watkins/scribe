"""Fails when Scribe's log says its engine and Tauri resolved different folders.

At startup the app resolves its data folders twice, once through Tauri and once
in the engine (`crates/scribe-core/src/paths.rs`), and logs
`The engine's folder differs from Tauri's: ...` for each difference
(`app/src-tauri/src/lib.rs`). A difference means user data could be written to
one folder and deleted from another, and the app keeps running, so only a check
of the log notices it.

Shared by the Windows end-to-end runner (windows/run.py) and the Linux and
macOS install-and-launch scripts (.github/scripts/smoke-*.sh). Standard library
only.

Command line, for the smoke scripts: pass log files or folders of `*.log` files.
Exit 0 when no line matches, 1 when one does (the lines are printed), 2 when no
log text was found at all (a check with nothing to read is blind, so it fails).

    python3 folder_mismatch.py LOG_OR_DIR [LOG_OR_DIR ...]
"""

from __future__ import annotations

import sys
from pathlib import Path

# Must match the message in app/src-tauri/src/lib.rs.
MISMATCH_MARKER = "The engine's folder differs from Tauri's"


def mismatch_lines(text: str) -> list[str]:
    """The log lines that report a folder disagreement."""
    return [line.strip() for line in text.splitlines() if MISMATCH_MARKER in line]


def read_logs(paths: list[Path]) -> str:
    """The text of the given log files, and of every `*.log` in the given folders."""
    parts = []
    for path in paths:
        files = sorted(path.glob("*.log")) if path.is_dir() else [path]
        for file in files:
            try:
                parts.append(file.read_text(encoding="utf-8", errors="replace"))
            except OSError:
                pass
    return "\n".join(parts)


def check_log(text: str) -> None:
    """Raises AssertionError, with the offending lines, when the log has a mismatch
    or has no text to check."""
    if not text.strip():
        raise AssertionError("The app log is empty or missing, so the folder check has nothing to read")
    lines = mismatch_lines(text)
    if lines:
        shown = "\n".join(f"  {line}" for line in lines)
        raise AssertionError(f"Scribe logged {len(lines)} folder mismatch(es):\n{shown}")


def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    text = read_logs([Path(arg) for arg in argv])
    try:
        check_log(text)
    except AssertionError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1 if mismatch_lines(text) else 2
    print("No folder mismatch in the app log.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
