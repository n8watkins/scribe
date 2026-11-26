"""Proves the folder-mismatch check can fail: run with `python3 -m unittest discover -s app/e2e`."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import folder_mismatch

SCRIPT = Path(folder_mismatch.__file__)
GOOD = "2026-11-26 [INFO] hotkey bindings registered\n2026-11-26 [INFO] ready\n"
BAD = (
    GOOD
    + "2026-11-26 [ERROR][scribe_lib] The engine's folder differs from Tauri's: data: C:\\a != C:\\b\n"
)


class FolderMismatchTest(unittest.TestCase):
    def test_clean_log_passes(self) -> None:
        folder_mismatch.check_log(GOOD)

    def test_log_with_the_line_fails_and_names_it(self) -> None:
        with self.assertRaises(AssertionError) as caught:
            folder_mismatch.check_log(BAD)
        self.assertIn("data: C:\\a != C:\\b", str(caught.exception))

    def test_empty_log_fails_because_the_check_would_be_blind(self) -> None:
        with self.assertRaises(AssertionError):
            folder_mismatch.check_log("  \n")

    def test_only_the_marker_matches(self) -> None:
        self.assertEqual(folder_mismatch.mismatch_lines("folder differs\nengine ok"), [])

    def test_marker_is_still_the_message_the_app_logs(self) -> None:
        source = SCRIPT.parents[2] / "app" / "src-tauri" / "src" / "lib.rs"
        self.assertIn(f'"{folder_mismatch.MISMATCH_MARKER}:', source.read_text(encoding="utf-8"))

    def run_cli(self, *logs: str) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as folder:
            for index, text in enumerate(logs):
                (Path(folder) / f"scribe{index}.log").write_text(text, encoding="utf-8")
            return subprocess.run(
                [sys.executable, "-I", str(SCRIPT), folder], capture_output=True, text=True, check=False
            )

    def test_cli_exit_codes(self) -> None:
        self.assertEqual(self.run_cli(GOOD).returncode, 0)
        self.assertEqual(self.run_cli(GOOD, BAD).returncode, 1)
        self.assertEqual(self.run_cli().returncode, 2)
        self.assertIn("folder mismatch", self.run_cli(BAD).stderr)


if __name__ == "__main__":
    unittest.main()
