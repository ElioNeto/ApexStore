"""Exercise the formatter CLI on Criterion output and publication failures."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

FORMATTER = Path(__file__).with_name('format_bench.py').resolve()


class FormatBenchTests(unittest.TestCase):
    def run_formatter(self, text):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / 'bench_output.txt').write_text(text)
        result = subprocess.run([sys.executable, str(FORMATTER)], cwd=root,
                                capture_output=True, text=True)
        return result, root

    def test_inline_and_multiline_criterion_measurements(self):
        result, root = self.run_formatter(
            'write_single/100 time:   [1.000 ns 2.000 ns 3.000 ns]\n'
            'read_memtable/100\n'
            '                        time:   [1.000 µs 2.000 µs 3.000 µs]\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        table = (root / 'bench_table.txt').read_text()
        self.assertIn('`write_single/100` | 2.000\u202fns', table)
        self.assertIn('`read_memtable/100` | 2.000\u202fµs', table)

    def test_empty_or_non_measurement_output_fails_without_publishing(self):
        for text in ['', 'Finished bench profile\nTesting write_single\nSuccess\n']:
            with self.subTest(text=text):
                result, root = self.run_formatter(text)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root / 'bench_table.txt').exists())

    def test_malformed_measurement_fails(self):
        result, root = self.run_formatter('write_single time: [1.0 ns]\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((root / 'bench_table.txt').exists())
