"""Locate the `adept` binary bundled inside this distribution's wheel data.

This is a derivative of ruff's `find_ruff_bin`, not a near-copy: it keeps
upstream's probe-then-return shape and ordering where the layouts overlap,
but diverges in a few places to fix bugs and cover layouts ruff doesn't need
to. Each divergence is called out in its own comment, so a future re-sync
against ruff does not blindly revert these fixes.
"""

import os
import sys
import sysconfig


class AdeptNotFound(FileNotFoundError):
    pass


def _is_under(root: str, path: str) -> bool:
    """Return whether `path` is `root` or nested inside it."""
    root = os.path.normcase(os.path.normpath(root))
    path = os.path.normcase(os.path.normpath(path))
    try:
        return os.path.commonpath([root, path]) == root
    except ValueError:
        # Different drives on Windows: never under the same root.
        return False


def find_adept_bin() -> str:
    """Return the path to the `adept` binary installed by this package.

    Probes every scripts directory a wheel installer might have used, in
    order, so the lookup works whether adept was installed with `uv tool
    install`, `pip install`, `pip install --target`, or `uv run --with`. The
    walk up from the package root is bounded so it can never escape the
    installation root onto an unrelated system-wide `adept`.
    """
    adept_exe = "adept" + (sysconfig.get_config_var("EXE") or "")

    candidates = []

    scripts_path = os.path.join(sysconfig.get_path("scripts"), adept_exe)
    candidates.append(scripts_path)
    if os.path.isfile(scripts_path):
        return scripts_path

    # Search from the base prefix, for `pip install --prefix ...` and similar.
    base_prefix_paths = (
        os.path.join(sys.base_prefix, "Scripts", adept_exe),
        os.path.join(sys.base_prefix, "bin", adept_exe),
    )
    for path in base_prefix_paths:
        candidates.append(path)
        if os.path.isfile(path):
            return path

    package_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    # Search in `bin`/`Scripts` adjacent to the package root: `pip install
    # --target X` puts the package at `X/adept` and the script at `X/bin`.
    # This is checked before the upward walk below (not upstream's order)
    # because it is the more specific match for that layout.
    for candidate in ("bin", "Scripts"):
        target_path = os.path.join(package_root, candidate, adept_exe)
        candidates.append(target_path)
        if os.path.isfile(target_path):
            return target_path

    # Search up the tree from the package root, bounded by containment
    # rather than a fixed step count (not upstream's approach): covers
    # `lib/python*/site-packages/adept` installs (`pip install --prefix
    # ...`, `uv run --with ...`), stepping up to the venv's `bin`/`Scripts`
    # directory, without ever escaping the installation root onto a
    # system-wide `adept` (e.g. `/usr/bin`).
    if _is_under(sys.prefix, package_root):
        install_root = sys.prefix
    else:
        # sys.prefix doesn't cover this install: derive the root from the
        # layout instead of guessing a depth. Find the nearest `site-packages`
        # or `dist-packages` ancestor — that's the standard
        # `<prefix>/lib/pythonX.Y/site-packages` (POSIX) or
        # `<prefix>/Lib/site-packages` (Windows) layout — and strip off the
        # `lib`/`lib64`/`Lib` (and, on POSIX, the `pythonX.Y`) segments above
        # it to land back on `<prefix>`. Absent such an ancestor, the only
        # layout left is `pip install --target X`, which has nothing above
        # the package worth walking to (its `bin` is a sibling, not an
        # ancestor's child); `install_root = package_root` makes the walk
        # below a no-op, which is correct — the adjacent-`bin` probe above
        # already covers that case.
        install_root = package_root
        directory = package_root
        while True:
            if os.path.normcase(os.path.basename(directory)) in (
                "site-packages",
                "dist-packages",
            ):
                lib_dir = os.path.dirname(directory)
                if os.path.normcase(os.path.basename(lib_dir)) in ("lib", "lib64"):
                    install_root = os.path.dirname(lib_dir)
                else:
                    # A `pythonX.Y`-style directory sits between `lib` and
                    # `site-packages`; step over it too.
                    install_root = os.path.dirname(os.path.dirname(lib_dir))
                break
            parent = os.path.dirname(directory)
            if parent == directory:
                break
            directory = parent
    directory = package_root
    while True:
        parent = os.path.dirname(directory)
        if parent == directory or not _is_under(install_root, parent):
            break
        for candidate in ("bin", "Scripts"):
            candidate_path = os.path.join(parent, candidate, adept_exe)
            candidates.append(candidate_path)
            if os.path.isfile(candidate_path):
                return candidate_path
        directory = parent

    # `sysconfig.get_preferred_scheme` covers every per-platform user-scheme
    # branch this used to hand-roll, now that the floor was raised to
    # Python 3.10.
    user_scheme = sysconfig.get_preferred_scheme("user")

    user_path = os.path.join(sysconfig.get_path("scripts", scheme=user_scheme), adept_exe)
    candidates.append(user_path)
    if os.path.isfile(user_path):
        return user_path

    raise AdeptNotFound(
        os.linesep.join(
            [
                "Unable to find the `adept` binary. Looked in the following locations:",
                *candidates,
            ]
        )
    )
