"""Locate the `adept` binary bundled inside this distribution's wheel data.

Rather than inferring an install layout from directory-name patterns (venv vs.
`--target` vs. `--prefix`, and the various scheme directory names each of
those uses), this reads the installing distribution's own `RECORD` via
`importlib.metadata` and resolves the script entry it names. The installer
already wrote down where it placed the binary, so there is nothing left to
guess.
"""

import os
import sysconfig
from importlib.metadata import PackageNotFoundError, distribution


class AdeptNotFound(FileNotFoundError):
    pass


def _is_script_entry(relative_path: str, exe_name: str) -> bool:
    """Return whether a `RECORD` entry is the `adept` script, not package data.

    `RECORD` also lists the `adept/` package's own files (including
    `__pycache__/*.pyc`) and the `*.dist-info/` metadata directory, so a
    same-named file can appear there too; both are excluded on their leading
    path segment. `relative_path` comes from a `PackagePath` (a
    `PurePosixPath` parsed from RECORD), so it is always `/`-separated, even
    on Windows.
    """
    if os.path.basename(relative_path) != exe_name:
        return False
    first_segment = relative_path.split("/", 1)[0]
    return first_segment != "adept" and not first_segment.endswith(".dist-info")


def find_adept_bin() -> str:
    """Return the path to the `adept` binary installed by this package.

    Reads the `adept` distribution's own `RECORD` (every install layout —
    venv, `--target`, `--prefix`, `uv tool install`, ... — records where it
    put the script) and resolves that entry to a filesystem path. Raises
    `AdeptNotFound` for each of the distinct ways that lookup can fail; there
    is no fallback to a guessed path.
    """
    exe_name = "adept" + (sysconfig.get_config_var("EXE") or "")

    try:
        dist = distribution("adept")
    except PackageNotFoundError as exc:
        raise AdeptNotFound(
            "No installed `adept` distribution was found via importlib.metadata "
            "(e.g. running from a source checkout rather than an installed wheel)."
        ) from exc

    if dist.files is None:
        raise AdeptNotFound(
            "The installed `adept` distribution has no RECORD, so the binary's "
            "location cannot be resolved."
        )

    entry = next((f for f in dist.files if _is_script_entry(str(f), exe_name)), None)
    if entry is None:
        raise AdeptNotFound(
            f"No `{exe_name}` script entry was found among the {len(dist.files)} "
            "RECORD entries for the `adept` distribution."
        )

    resolved = os.path.normpath(dist.locate_file(entry))
    if not os.path.isfile(resolved):
        raise AdeptNotFound(
            f"RECORD names `{resolved}` as the `adept` binary, but no file exists there."
        )

    return resolved
