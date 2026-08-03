"""Locate the `adept` binary bundled inside this distribution's wheel data.

Rather than inferring an install layout from directory-name patterns (venv vs.
`--target` vs. `--prefix`, and the various scheme directory names each of
those uses), this reads the installing distribution's own `RECORD` via
`importlib.metadata` and resolves the script entry it names. The installer
already wrote down where it placed the binary, so there is nothing left to
guess.

Matching that entry on its basename is the deepest identification available:
`RECORD` has no column saying which scheme a file went to, the `*.data/scripts/`
prefix that carried that meaning in the wheel is rewritten away at install
time, and a maturin `bindings = "bin"` binary is a data script rather than a
`console_scripts` entry point, so `entry_points` is empty.
"""

import os
import sys
from importlib.metadata import PackageNotFoundError, PackagePath, distribution

_REMEDY = "Reinstall adept from a wheel, e.g. `pip install adept`."


class AdeptNotFound(FileNotFoundError):
    pass


def _is_script_entry(entry: PackagePath, exe_name: str) -> bool:
    """Return whether a `RECORD` entry is the `adept` script, not package data.

    The exclusions cover the two places a same-named file could otherwise
    appear: the `adept/` package itself and the `*.dist-info/` metadata
    directory.
    """
    if entry.name != exe_name:
        return False
    first_segment = entry.parts[0]
    return first_segment != "adept" and not first_segment.endswith(".dist-info")


def find_adept_bin() -> str:
    """Return the path to the `adept` binary installed by this package."""
    exe_name = "adept" + (".exe" if sys.platform == "win32" else "")

    try:
        dist = distribution("adept")
    except PackageNotFoundError as exc:
        raise AdeptNotFound(
            "No installed `adept` distribution was found via importlib.metadata "
            "(e.g. running from a source checkout rather than an installed "
            f"wheel). {_REMEDY}"
        ) from exc

    # `files` re-reads and re-parses RECORD on every access, so bind it once.
    files = dist.files
    if files is None:
        raise AdeptNotFound(
            "The installed `adept` distribution has no RECORD, so the binary's "
            f"location cannot be resolved. {_REMEDY}"
        )

    entry = next((f for f in files if _is_script_entry(f, exe_name)), None)
    if entry is None:
        raise AdeptNotFound(
            f"No `{exe_name}` script entry was found among the {len(files)} "
            f"RECORD entries for the `adept` distribution. {_REMEDY}"
        )

    resolved = os.path.normpath(entry.locate())
    if not os.path.isfile(resolved):
        # Only reachable on Python 3.10/3.11: from 3.12 `Distribution.files`
        # drops entries whose file is missing, so this surfaces as the
        # no-matching-entry case above instead.
        raise AdeptNotFound(
            f"RECORD names `{resolved}` as the `adept` binary, but no file "
            f"exists there. {_REMEDY}"
        )

    return resolved
