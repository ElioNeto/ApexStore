"""Exercise publication through the CLI, including no-op and failure cases."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

UPDATER = Path(__file__).with_name("update_benchmark_report.py").resolve()
START = "<!-- BENCHMARK_RESULTS_START -->"
END = "<!-- BENCHMARK_RESULTS_END -->"
TABLE = "**Read**\n\n| Benchmark | Median | Perf |\n|---|---|---|\n| `read/100` | 2.0 µs | green |"


class UpdateReportTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.report = Path(directory.name) / "PERFORMANCE.md"
        self.table = Path(directory.name) / "table.md"
        self.report.write_text(f"Manual introduction\n{START}\nPending\n{END}\nManual conclusion\n")
        self.table.write_text(TABLE)

    def run_update(self, run="first", commit="abc"):
        return subprocess.run(
            [sys.executable, str(UPDATER), "--report", str(self.report),
             "--table", str(self.table), "--run-url", f"https://example.test/{run}",
             "--commit", commit, "--runner", "Linux"], capture_output=True, text=True)

    def test_updates_measurements_and_preserves_surrounding_document(self):
        result = self.run_update()
        self.assertEqual(result.returncode, 0, result.stderr)
        text = self.report.read_text()
        self.assertTrue(text.startswith("Manual introduction\n"))
        self.assertTrue(text.endswith("Manual conclusion\n"))
        self.assertIn(TABLE, text)
        self.assertIn("https://example.test/first", text)
        self.assertIn("Commit: `abc`", text)

        self.table.write_text(TABLE.replace("2.0 µs", "3.0 µs"))
        self.assertEqual(self.run_update("second", "def").returncode, 0)
        self.assertIn("3.0 µs", self.report.read_text())
        self.assertIn("Commit: `def`", self.report.read_text())

    def test_identical_table_does_not_write_or_replace_provenance(self):
        self.assertEqual(self.run_update().returncode, 0)
        original = self.report.read_bytes()
        os.utime(self.report, (1, 1))
        self.table.write_text("\n" + TABLE + "\n\n")
        result = self.run_update("different-run", "different-sha")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.report.read_bytes(), original)
        self.assertEqual(self.report.stat().st_mtime_ns, 1_000_000_000)

    def test_empty_or_placeholder_output_cannot_replace_report(self):
        original = self.report.read_bytes()
        for table in ["", "   ", "No results parsed", "| Benchmark | Median |\n|---|---|"]:
            with self.subTest(table=table):
                self.table.write_text(table)
                self.assertNotEqual(self.run_update().returncode, 0)
                self.assertEqual(self.report.read_bytes(), original)

    def test_missing_duplicate_or_reversed_markers_fail_without_writing(self):
        for text in ["no markers", START, f"{END}\n{START}", f"{START}\n{START}\n{END}"]:
            with self.subTest(text=text):
                self.report.write_text(text)
                self.assertNotEqual(self.run_update().returncode, 0)
                self.assertEqual(self.report.read_text(), text)


if __name__ == "__main__":
    unittest.main()
