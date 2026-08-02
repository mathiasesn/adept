"""Locate the `adept` binary bundled inside this distribution's wheel data."""

import os
import sys
import sysconfig


class AdeptNotFound(FileNotFoundError):
    pass


def find_adept_bin() -> str:
    """Return the path to the `adept` binary installed by this package.

    Adapted from ruff's `find_ruff_bin`: probes every scripts directory a
    wheel installer might have used, in order, so the lookup works whether
    adept was installed with `uv tool install`, `pip install`,
    `pip install --target`, or `uv run --with`.

    The probe-then-return repetition is upstream's shape and is kept
    deliberately, so this stays diffable against ruff on a re-sync. One part
    is *not* upstream's: the bounded upward walk below replaces ruff's single
    `site-packages`-relative probe, to cover layouts where the binary sits
    more than one level above the package.
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

    # Search up the tree from the package root, bounded to a fixed number of
    # steps so we cannot escape the installation tree onto a system-wide
    # `adept`: covers `lib/python*/site-packages/adept` installs (`pip
    # install --prefix ...`, `uv run --with ...`), stepping up to the venv's
    # `bin`/`Scripts` directory.
    package_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    directory = package_root
    for _ in range(6):
        parent = os.path.dirname(directory)
        if parent == directory:
            break
        for candidate in ("bin", "Scripts"):
            candidate_path = os.path.join(parent, candidate, adept_exe)
            candidates.append(candidate_path)
            if os.path.isfile(candidate_path):
                return candidate_path
        directory = parent

    # Search in `bin` adjacent to package root (e.g. `pip install --target ...`).
    target_path = os.path.join(package_root, adept_exe)
    candidates.append(target_path)
    if os.path.isfile(target_path):
        return target_path

    if sys.version_info >= (3, 10):
        user_scheme = sysconfig.get_preferred_scheme("user")
    elif os.name == "nt":
        user_scheme = "nt_user"
    elif sys.platform == "darwin" and sys._framework:
        user_scheme = "osx_framework_user"
    else:
        user_scheme = "posix_user"

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
