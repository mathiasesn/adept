"""Locate the `adept` binary bundled inside this distribution's wheel data."""

import os
import sys
import sysconfig


class AdeptNotFound(FileNotFoundError):
    pass


def find_adept_bin() -> str:
    """Return the path to the `adept` binary installed by this package.

    Adapted from ruff's `find_ruff_bin` implementation: probes every
    scripts directory a wheel installer might have used, in order, so the
    lookup works whether adept was installed with `uv tool install`,
    `pip install`, `pip install --target`, or `uv run --with`.
    """
    adept_exe = "adept" + sysconfig.get_config_var("EXE") if sysconfig.get_config_var("EXE") else "adept"

    scripts_path = os.path.join(sysconfig.get_path("scripts"), adept_exe)
    if os.path.isfile(scripts_path):
        return scripts_path

    if sys.version_info >= (3, 10):
        user_scheme = sysconfig.get_preferred_scheme("user")
    elif os.name == "nt":
        user_scheme = "nt_user"
    elif sys.platform == "darwin" and sys._framework:
        user_scheme = "osx_framework_user"
    else:
        user_scheme = "posix_user"

    user_path = os.path.join(sysconfig.get_path("scripts", scheme=user_scheme), adept_exe)
    if os.path.isfile(user_path):
        return user_path

    # Search from the base prefix, for `pip install --prefix ...` and similar.
    paths = (
        os.path.join(sys.base_prefix, "Scripts", adept_exe),
        os.path.join(sys.base_prefix, "bin", adept_exe),
    )
    for path in paths:
        if os.path.isfile(path):
            return path

    # Search in `bin` adjacent to package root (e.g. `pip install --target ...`).
    package_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    target_path = os.path.join(package_root, adept_exe)
    if os.path.isfile(target_path):
        return target_path

    # Search up the tree from the package root: covers `lib/python*/site-packages/adept`
    # installs, stepping up to the venv's `bin`/`Scripts` directory.
    directory = package_root
    while directory != os.path.dirname(directory):
        for candidate in ("bin", "Scripts"):
            candidate_path = os.path.join(directory, candidate, adept_exe)
            if os.path.isfile(candidate_path):
                return candidate_path
        directory = os.path.dirname(directory)

    raise AdeptNotFound(
        os.linesep.join(
            [
                "Unable to find the `adept` binary. Looked in the following locations:",
                scripts_path,
                user_path,
                *paths,
                target_path,
            ]
        )
    )
