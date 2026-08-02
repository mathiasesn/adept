"""Pin the binary-discovery ordering of `adept._find_adept.find_adept_bin`.

Every case builds a fake install tree rooted at `tmp_path` and monkeypatches
`sysconfig.get_path`, `sys.base_prefix`, `sys.prefix`, and the module's
`__file__` so the walk never touches the real filesystem. In particular, no
case may ever return a decoy binary planted outside the "correct" location,
and the no-binary case must raise `AdeptNotFound` rather than fall through to
a real system-wide `adept`.
"""

import os
import stat
import sys
from pathlib import Path

import pytest

from adept import _find_adept
from adept._find_adept import AdeptNotFound, find_adept_bin

EXE_NAME = "adept" + (".exe" if sys.platform == "win32" else "")


def _make_binary(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\necho fake adept\n")
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


def _patch_tree(
    monkeypatch: pytest.MonkeyPatch,
    *,
    scripts_dir: Path,
    base_prefix: Path,
    prefix: Path,
    package_file: Path,
) -> None:
    """Point every input `find_adept_bin` reads at a fake, tmp_path-rooted tree."""

    real_get_path = _find_adept.sysconfig.get_path
    user_scripts_dir = scripts_dir.parent / "user-scheme-scripts-empty"

    def fake_get_path(name, scheme=None):
        if name == "scripts" and scheme is None:
            return str(scripts_dir)
        if name == "scripts":
            # The final user-scheme probe: keep it inside tmp_path and empty,
            # so it can never accidentally resolve to a real file on disk.
            return str(user_scripts_dir)
        # Anything else (e.g. "stdlib", consulted internally by
        # sysconfig.get_config_var("EXE")) is safe to resolve for real: it
        # never influences which binary is returned.
        return real_get_path(name, scheme=scheme) if scheme is not None else real_get_path(name)

    monkeypatch.setattr(_find_adept.sysconfig, "get_path", fake_get_path)
    monkeypatch.setattr(_find_adept.sysconfig, "get_preferred_scheme", lambda kind: "posix_prefix")
    monkeypatch.setattr(_find_adept.sys, "base_prefix", str(base_prefix))
    monkeypatch.setattr(_find_adept.sys, "prefix", str(prefix))
    # __file__ is python/adept/_find_adept.py; package_root is dirname(dirname(__file__)).
    # So package_root == package_file.parent.parent.
    monkeypatch.setattr(_find_adept, "__file__", str(package_file))


def test_scripts_dir_hit_returns_immediately(tmp_path, monkeypatch):
    """venv layout: sysconfig.get_path("scripts") holds the binary -> first probe wins."""
    scripts_dir = tmp_path / "venv" / "bin"
    binary = scripts_dir / EXE_NAME
    _make_binary(binary)

    # Package root somewhere unrelated and empty, so only the scripts probe could succeed.
    package_file = tmp_path / "venv" / "lib" / "python3.12" / "site-packages" / "adept" / "_find_adept.py"
    package_file.parent.mkdir(parents=True, exist_ok=True)

    _patch_tree(
        monkeypatch,
        scripts_dir=scripts_dir,
        base_prefix=tmp_path / "unused-base",
        prefix=tmp_path / "venv",
        package_file=package_file,
    )

    assert find_adept_bin() == str(binary)


def test_base_prefix_bin_hit(tmp_path, monkeypatch):
    """`pip install --prefix` layout -> found via sys.base_prefix/bin."""
    scripts_dir = tmp_path / "scripts-empty"
    base_prefix = tmp_path / "prefix-root"
    binary = base_prefix / "bin" / EXE_NAME
    _make_binary(binary)

    package_file = base_prefix / "lib" / "python3.12" / "site-packages" / "adept" / "_find_adept.py"
    package_file.parent.mkdir(parents=True, exist_ok=True)

    _patch_tree(
        monkeypatch,
        scripts_dir=scripts_dir,
        base_prefix=base_prefix,
        prefix=base_prefix,
        package_file=package_file,
    )

    assert find_adept_bin() == str(binary)


def test_upward_walk_with_derived_root(tmp_path, monkeypatch):
    """`uv run --with` layout: sys.prefix does NOT cover the package, forcing the
    derived-root path via the nearest site-packages ancestor, and the binary sits
    at <derived-root>/bin/adept, found only by the upward walk.
    """
    install_root = tmp_path / "derived-root"
    package_file = install_root / "lib" / "python3.12" / "site-packages" / "adept" / "_find_adept.py"
    package_file.parent.mkdir(parents=True, exist_ok=True)
    binary = install_root / "bin" / EXE_NAME
    _make_binary(binary)

    scripts_dir = tmp_path / "scripts-empty"
    base_prefix = tmp_path / "base-prefix-empty"
    # sys.prefix points somewhere entirely unrelated to package_root, so
    # _is_under(sys.prefix, package_root) is False and the derived-root branch runs.
    prefix = tmp_path / "unrelated-prefix"

    _patch_tree(
        monkeypatch,
        scripts_dir=scripts_dir,
        base_prefix=base_prefix,
        prefix=prefix,
        package_file=package_file,
    )

    assert find_adept_bin() == str(binary)


def test_target_install_adjacent_bin_wins_over_decoys(tmp_path, monkeypatch):
    """`pip install --target X`: package at X/adept, correct binary at X/bin/adept.
    Decoys at <tmp>/bin/adept and <tmp>/opt/bin/adept must not be returned.
    """
    target = tmp_path / "X"
    package_file = target / "adept" / "_find_adept.py"
    package_file.parent.mkdir(parents=True, exist_ok=True)
    correct_binary = target / "bin" / EXE_NAME
    _make_binary(correct_binary)

    decoy1 = tmp_path / "bin" / EXE_NAME
    _make_binary(decoy1)
    decoy2 = tmp_path / "opt" / "bin" / EXE_NAME
    _make_binary(decoy2)

    scripts_dir = tmp_path / "scripts-empty"
    base_prefix = tmp_path / "base-prefix-empty"
    prefix = tmp_path / "prefix-empty"

    _patch_tree(
        monkeypatch,
        scripts_dir=scripts_dir,
        base_prefix=base_prefix,
        prefix=prefix,
        package_file=package_file,
    )

    result = find_adept_bin()
    assert result == str(correct_binary)
    assert result != str(decoy1)
    assert result != str(decoy2)


def test_no_binary_raises_and_ignores_decoy(tmp_path, monkeypatch):
    """dist-packages-shaped tree with no binary anywhere inside the install root
    must raise AdeptNotFound, never falling through to a decoy at <tmp>/bin/adept
    (which sits outside the derived install root).
    """
    install_root = tmp_path / "derived-root"
    package_file = install_root / "lib" / "python3.12" / "dist-packages" / "adept" / "_find_adept.py"
    package_file.parent.mkdir(parents=True, exist_ok=True)

    # Decoy sits above/outside the derived install root: must never be reached.
    decoy = tmp_path / "bin" / EXE_NAME
    _make_binary(decoy)

    scripts_dir = tmp_path / "scripts-empty"
    base_prefix = tmp_path / "base-prefix-empty"
    prefix = tmp_path / "unrelated-prefix"

    _patch_tree(
        monkeypatch,
        scripts_dir=scripts_dir,
        base_prefix=base_prefix,
        prefix=prefix,
        package_file=package_file,
    )

    with pytest.raises(AdeptNotFound):
        find_adept_bin()


def test_target_install_no_binary_raises_despite_decoys_above(tmp_path, monkeypatch):
    """`--target`-shaped tree with no binary anywhere, and decoys above the
    package root, must still raise AdeptNotFound rather than returning a decoy.
    """
    target = tmp_path / "X"
    package_file = target / "adept" / "_find_adept.py"
    package_file.parent.mkdir(parents=True, exist_ok=True)

    decoy1 = tmp_path / "bin" / EXE_NAME
    _make_binary(decoy1)
    decoy2 = tmp_path / "opt" / "bin" / EXE_NAME
    _make_binary(decoy2)

    scripts_dir = tmp_path / "scripts-empty"
    base_prefix = tmp_path / "base-prefix-empty"
    prefix = tmp_path / "prefix-empty"

    _patch_tree(
        monkeypatch,
        scripts_dir=scripts_dir,
        base_prefix=base_prefix,
        prefix=prefix,
        package_file=package_file,
    )

    with pytest.raises(AdeptNotFound):
        find_adept_bin()
