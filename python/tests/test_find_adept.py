"""Pin `adept._find_adept.find_adept_bin` against RECORD-based discovery.

Every case builds a fake `RECORD` file list and injects it by patching
`_find_adept.distribution`, rather than touching the real installed `adept`
distribution or the real filesystem outside `tmp_path`. In particular, no case
may ever return a decoy binary that sits outside what RECORD actually names,
and the failure-mode cases must raise `AdeptNotFound` rather than fall through
to a guessed path.
"""

import stat
import sysconfig
from pathlib import Path

import pytest
from importlib.metadata import PackageNotFoundError

from adept import _find_adept
from adept._find_adept import AdeptNotFound, find_adept_bin

# Derived exactly as the code under test derives it, so the tests stay
# coupled to the implementation rather than to the host platform.
EXE_NAME = "adept" + (sysconfig.get_config_var("EXE") or "")


def _make_binary(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\necho fake adept\n")
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


class _FakeDistribution:
    """Stands in for `importlib.metadata.Distribution` in tests.

    `files` holds RECORD-style relative path strings; `locate_file` resolves
    them against `root`, mirroring how a real distribution's `locate_file`
    resolves RECORD entries (including `../../../bin/adept`-style entries)
    against its own base directory.
    """

    def __init__(self, root: Path, files: list[str] | None) -> None:
        self.root = root
        self.files = files

    def locate_file(self, path: str) -> str:
        return str(self.root / path)


def _patch_distribution(monkeypatch: pytest.MonkeyPatch, dist: object) -> None:
    def fake_distribution(name: str) -> object:
        assert name == "adept"
        return dist

    monkeypatch.setattr(_find_adept, "distribution", fake_distribution)


def _raise_not_found(name: str) -> None:
    raise PackageNotFoundError(name)


# --- the two real install shapes verified against actual wheels -----------


def test_venv_install_resolves_relative_record_entry(tmp_path, monkeypatch):
    """`uv pip install` into a venv: RECORD holds `../../../bin/adept` relative
    to the `site-packages` dist-info directory; locate_file + normpath must
    resolve it to the venv's `bin/adept`.
    """
    site_packages = tmp_path / "venv" / "lib" / "python3.12" / "site-packages"
    binary = tmp_path / "venv" / "bin" / EXE_NAME
    _make_binary(binary)

    dist = _FakeDistribution(
        root=site_packages,
        files=[
            "adept/__init__.py",
            "adept/__pycache__/_find_adept.cpython-312.pyc",
            "adept-0.1.0.dist-info/RECORD",
            "adept-0.1.0.dist-info/METADATA",
            f"../../../bin/{EXE_NAME}",
        ],
    )
    _patch_distribution(monkeypatch, dist)

    assert find_adept_bin() == str(binary)


def test_target_install_resolves_sibling_bin_entry(tmp_path, monkeypatch):
    """`uv pip install --target X`: RECORD holds `bin/adept` relative to X."""
    target = tmp_path / "X"
    binary = target / "bin" / EXE_NAME
    _make_binary(binary)

    dist = _FakeDistribution(
        root=target,
        files=[
            "adept/__init__.py",
            "adept-0.1.0.dist-info/RECORD",
            f"bin/{EXE_NAME}",
        ],
    )
    _patch_distribution(monkeypatch, dist)

    assert find_adept_bin() == str(binary)


# --- the four failure modes ------------------------------------------------


def test_no_distribution_raises(monkeypatch):
    """Running from a source tree / not installed: distribution() raises."""

    def fake_distribution(name: str) -> object:
        _raise_not_found(name)

    monkeypatch.setattr(_find_adept, "distribution", fake_distribution)

    with pytest.raises(AdeptNotFound, match="No installed `adept` distribution"):
        find_adept_bin()


def test_no_record_raises(tmp_path, monkeypatch):
    """Distribution exists but dist.files is None (no RECORD)."""
    dist = _FakeDistribution(root=tmp_path, files=None)
    _patch_distribution(monkeypatch, dist)

    with pytest.raises(AdeptNotFound, match="no RECORD"):
        find_adept_bin()


def test_no_matching_entry_raises(tmp_path, monkeypatch):
    """RECORD exists but contains no script entry matching the selection rule."""
    dist = _FakeDistribution(
        root=tmp_path,
        files=[
            "adept/__init__.py",
            "adept/__pycache__/_find_adept.cpython-312.pyc",
            "adept-0.1.0.dist-info/RECORD",
            "adept-0.1.0.dist-info/METADATA",
        ],
    )
    _patch_distribution(monkeypatch, dist)

    with pytest.raises(AdeptNotFound, match="No .* script entry"):
        find_adept_bin()


def test_matched_entry_missing_file_raises(tmp_path, monkeypatch):
    """RECORD names a script entry, but no file exists at the resolved path."""
    dist = _FakeDistribution(
        root=tmp_path,
        files=[
            "adept/__init__.py",
            f"bin/{EXE_NAME}",
        ],
    )
    _patch_distribution(monkeypatch, dist)

    with pytest.raises(AdeptNotFound, match="no file exists"):
        find_adept_bin()


# --- escape prevention ------------------------------------------------------


def test_decoy_outside_install_tree_is_never_returned(tmp_path, monkeypatch):
    """A binary sitting outside what RECORD names must never be returned, even
    when RECORD's own script entry does not resolve to a real file.
    """
    decoy = tmp_path / "somewhere-else" / "bin" / EXE_NAME
    _make_binary(decoy)

    dist = _FakeDistribution(
        root=tmp_path / "X",
        files=[
            "adept/__init__.py",
            f"bin/{EXE_NAME}",
        ],
    )
    _patch_distribution(monkeypatch, dist)

    with pytest.raises(AdeptNotFound):
        find_adept_bin()


def test_decoy_under_package_or_dist_info_is_never_returned(tmp_path, monkeypatch):
    """A same-named file under `adept/` or `*.dist-info/` must be excluded by
    the selection rule even though it matches on basename, and the real
    script entry elsewhere in RECORD must still win.
    """
    decoy_in_package = tmp_path / "adept" / EXE_NAME
    _make_binary(decoy_in_package)
    decoy_in_dist_info = tmp_path / "adept-0.1.0.dist-info" / EXE_NAME
    _make_binary(decoy_in_dist_info)
    correct_binary = tmp_path / "bin" / EXE_NAME
    _make_binary(correct_binary)

    dist = _FakeDistribution(
        root=tmp_path,
        files=[
            f"adept/{EXE_NAME}",
            f"adept-0.1.0.dist-info/{EXE_NAME}",
            f"bin/{EXE_NAME}",
        ],
    )
    _patch_distribution(monkeypatch, dist)

    result = find_adept_bin()
    assert result == str(correct_binary)
    assert result != str(decoy_in_package)
    assert result != str(decoy_in_dist_info)
