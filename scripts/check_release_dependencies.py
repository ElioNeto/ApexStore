"""Prevent publishing a library whose tested dependency patches are not published."""
import sys
import tomllib
from pathlib import Path

manifest = Path(sys.argv[1] if len(sys.argv) > 1 else 'Cargo.toml')
config = tomllib.loads(manifest.read_text())
patches = config.get('patch', {})
if any(patches.values()):
    sys.exit('Release blocked: Cargo [patch] overrides are not inherited by library consumers. '
             'Replace the temporary Actix PR revision with a released native HTTP/2 version '
             'and rerun the full validation before publishing.')
print('Release dependencies have no root-only patches.')
