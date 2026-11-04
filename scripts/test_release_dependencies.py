import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CHECK = Path(__file__).with_name('check_release_dependencies.py').resolve()


class ReleaseDependenciesTests(unittest.TestCase):
    def test_registry_manifest_can_publish_but_patched_manifest_cannot(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / 'Cargo.toml'
            base = '[package]\nname = "fixture"\nversion = "1.0.0"\n'
            for extra, expected in [('', 0), ('[patch.crates-io]\nactix-web = { git = "https://github.com/actix/actix-web", rev = "fixed-revision" }\n', 1)]:
                with self.subTest(patched=bool(extra)):
                    manifest.write_text(base + extra)
                    result = subprocess.run([sys.executable, str(CHECK), str(manifest)], capture_output=True, text=True)
                    self.assertEqual(result.returncode, expected, result.stderr)
