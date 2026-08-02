"""Locate the `adept` binary bundled inside this distribution's wheel data.

Derived from ruff's `find_ruff_bin`, but no longer diffable against it: the
probe order changed, the upward walk is bounded by containment, and the
version-gated user-scheme cascade is gone. Treat this as its own code — a
re-sync against upstream would reintroduce bugs this file exists to fix.
"""

import os
import sys
import sysconfig
from typing import Iterator


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


def _install_root(package_root: str) -> str:
    """Return the highest directory the upward walk may reach.

    Bounding the walk is what keeps a failed lookup from escaping onto an
    unrelated system-wide `adept` in `/usr/bin` or `/bin`.
    """
    if _is_under(sys.prefix, package_root):
        return sys.prefix

    # sys.prefix doesn't cover this install, so derive the root from the
    # layout: the nearest `site-packages`/`dist-packages` ancestor sits at
    # `<root>/lib/pythonX.Y/site-packages` (POSIX) or `<root>/Lib/site-packages`
    # (Windows), so stripping those segments lands back on `<root>`.
    directory = package_root
    while True:
        if os.path.normcase(os.path.basename(directory)) in ("site-packages", "dist-packages"):
            lib_dir = os.path.dirname(directory)
            if os.path.normcase(os.path.basename(lib_dir)) in ("lib", "lib64"):
                return os.path.dirname(lib_dir)
            # A `pythonX.Y` directory sits between `lib` and `site-packages`.
            return os.path.dirname(os.path.dirname(lib_dir))
        parent = os.path.dirname(directory)
        if parent == directory:
            # No such ancestor, so this is `pip install --target X`, whose
            # `bin` is a sibling of the package rather than an ancestor's
            # child. Returning the package root makes the walk a no-op; the
            # adjacent-`bin` probe already covered that layout.
            return package_root
        directory = parent


def _candidate_paths(adept_exe: str) -> Iterator[str]:
    """Yield every location the binary might be in, in probe order.

    The order is load-bearing and pinned by `python/tests/test_find_adept.py`.
    """
    yield os.path.join(sysconfig.get_path("scripts"), adept_exe)

    # `pip install --prefix ...` and similar.
    yield os.path.join(sys.base_prefix, "Scripts", adept_exe)
    yield os.path.join(sys.base_prefix, "bin", adept_exe)

    package_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    # `pip install --target X` puts the package at `X/adept` and the script at
    # `X/bin`. Probed before the walk below: it is the more specific match.
    for name in ("bin", "Scripts"):
        yield os.path.join(package_root, name, adept_exe)

    # Walk up to the venv's `bin`/`Scripts`, covering
    # `lib/python*/site-packages/adept` installs (`pip install --prefix ...`,
    # `uv run --with ...`), stopping before the walk leaves the install root.
    install_root = _install_root(package_root)
    directory = package_root
    while True:
        parent = os.path.dirname(directory)
        if parent == directory or not _is_under(install_root, parent):
            break
        for name in ("bin", "Scripts"):
            yield os.path.join(parent, name, adept_exe)
        directory = parent

    # One call covers every per-platform user-scheme branch this used to
    # hand-roll, now that the floor is Python 3.10.
    user_scheme = sysconfig.get_preferred_scheme("user")
    yield os.path.join(sysconfig.get_path("scripts", scheme=user_scheme), adept_exe)


def find_adept_bin() -> str:
    """Return the path to the `adept` binary installed by this package.

    Probes every scripts directory a wheel installer might have used, so the
    lookup works whether adept was installed with `uv tool install`, `pip
    install`, `pip install --target`, or `uv run --with`. It never returns a
    binary outside this installation's tree.
    """
    adept_exe = "adept" + (sysconfig.get_config_var("EXE") or "")

    tried = []
    for path in _candidate_paths(adept_exe):
        tried.append(path)
        if os.path.isfile(path):
            return path

    raise AdeptNotFound(
        os.linesep.join(
            [
                "Unable to find the `adept` binary. Looked in the following locations:",
                *tried,
            ]
        )
    )
