from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path


DEFAULT_REPO = "VeraTools/Vera"
MAX_REDIRECTS = 5


def package_version() -> str:
    try:
        return version("vera-ai")
    except PackageNotFoundError:
        pyproject = Path(__file__).resolve().parents[2] / "pyproject.toml"
        for line in pyproject.read_text(encoding="utf-8").splitlines():
            if line.startswith("version = "):
                return line.split("=", 1)[1].strip().strip('"')
        raise


def _detect_musl() -> bool:
    if platform.system().lower() != "linux":
        return False
    try:
        result = subprocess.run(
            ["ldd", "--version"], capture_output=True, text=True, check=False,
        )
        combined = (result.stdout or "") + (result.stderr or "")
        if "musl" in combined.lower():
            return True
    except FileNotFoundError:
        pass
    try:
        return any(e.startswith("ld-musl-") for e in os.listdir("/lib"))
    except OSError:
        return False


def resolve_target() -> str:
    override = os.environ.get("VERA_TARGET")
    if override:
        return override

    system = platform.system().lower()
    machine = platform.machine().lower()

    linux_x86 = "x86_64-unknown-linux-musl" if _detect_musl() else "x86_64-unknown-linux-gnu"
    targets = {
        ("linux", "x86_64"): linux_x86,
        ("linux", "amd64"): linux_x86,
        ("linux", "aarch64"): "aarch64-unknown-linux-gnu",
        ("linux", "arm64"): "aarch64-unknown-linux-gnu",
        ("darwin", "x86_64"): "x86_64-apple-darwin",
        ("darwin", "arm64"): "aarch64-apple-darwin",
        ("windows", "amd64"): "x86_64-pc-windows-msvc",
        ("windows", "x86_64"): "x86_64-pc-windows-msvc",
    }

    try:
        return targets[(system, machine)]
    except KeyError as exc:
        raise RuntimeError(f"unsupported platform: {platform.system()}/{platform.machine()}") from exc


def default_release_base_url() -> str:
    return os.environ.get("VERA_RELEASE_BASE_URL", f"https://github.com/{DEFAULT_REPO}")


def manifest_url() -> str:
    return os.environ.get(
        "VERA_MANIFEST_URL",
        f"{default_release_base_url()}/releases/download/v{package_version()}/release-manifest.json",
    )


def latest_manifest_url() -> str:
    return f"{default_release_base_url()}/releases/latest/download/release-manifest.json"


def vera_home() -> Path:
    return Path(os.environ.get("VERA_HOME", Path.home() / ".vera")).expanduser()


def install_metadata_path() -> Path:
    return vera_home() / "install.json"


def detect_wrapper_install_method() -> str | None:
    explicit = os.environ.get("VERA_INSTALL_METHOD")
    if explicit in {"pip", "uv"}:
        return explicit

    if any(
        os.environ.get(name)
        for name in ("UV", "UV_CACHE_DIR", "UV_TOOL_DIR", "UV_TOOL_BIN_DIR")
    ):
        return "uv"

    return "pip"


def read_install_metadata() -> dict[str, object]:
    path = install_metadata_path()
    if not path.exists():
        return {}

    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return {}


def write_install_metadata(
    *,
    install_method: str | None,
    version_value: str | None,
    binary_path: Path | None,
    target: str | None = None,
) -> None:
    path = install_metadata_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    current = read_install_metadata()
    payload = {
        "install_method": install_method or current.get("install_method"),
        "version": version_value or current.get("version"),
        "binary_path": str(binary_path) if binary_path is not None else current.get("binary_path"),
        "target": target or current.get("target"),
    }
    tmp_path = path.with_suffix(f".tmp.{os.getpid()}")
    tmp_path.write_text(f"{json.dumps(payload, indent=2)}\n", encoding="utf-8")
    tmp_path.replace(path)


def preferred_bin_dirs() -> list[Path]:
    override = os.environ.get("VERA_USER_BIN_DIR")
    if override:
        return [Path(override).expanduser()]

    home = Path.home()
    if os.name == "nt":
        return [
            home / "AppData" / "Roaming" / "npm",
            home / "AppData" / "Local" / "Programs" / "Vera" / "bin",
        ]

    return [home / ".local" / "bin", home / ".cargo" / "bin", home / "bin"]


def path_entries() -> set[Path]:
    entries = os.environ.get("PATH", "").split(os.pathsep)
    return {Path(entry).expanduser().resolve() for entry in entries if entry}


def pick_user_bin_dir() -> Path:
    entries = path_entries()
    for candidate in preferred_bin_dirs():
        resolved = candidate.expanduser().resolve()
        if resolved in entries:
            return resolved
    return preferred_bin_dirs()[0].expanduser().resolve()


def binary_name() -> str:
    return "vera.exe" if os.name == "nt" else "vera"


def shim_name() -> str:
    return "vera.cmd" if os.name == "nt" else "vera"


def read_json(url: str) -> dict[str, object]:
    with urllib.request.urlopen(url) as response:
        return json.loads(response.read().decode("utf-8"))


def load_manifest() -> dict[str, object]:
    try:
        return read_json(manifest_url())
    except Exception:
        if os.environ.get("VERA_MANIFEST_URL"):
            raise
        return read_json(latest_manifest_url())


def download_file(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url) as response, destination.open("wb") as handle:
        shutil.copyfileobj(response, handle)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def extract_archive(archive_path: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    if archive_path.suffix == ".zip":
        with zipfile.ZipFile(archive_path) as archive:
            archive.extractall(destination)
        return

    with tarfile.open(archive_path, "r:gz") as archive:
        archive.extractall(destination)


def create_shim(binary_path: Path) -> Path:
    bin_dir = pick_user_bin_dir()
    bin_dir.mkdir(parents=True, exist_ok=True)
    shim_path = bin_dir / shim_name()

    if os.name == "nt":
        shim_path.write_text(f'@echo off\r\n"{binary_path}" %*\r\n', encoding="utf-8")
    else:
        shim_path.write_text(f'#!/bin/sh\nexec "{binary_path}" "$@"\n', encoding="utf-8")
        shim_path.chmod(shim_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    return shim_path


def ensure_binary_installed() -> tuple[Path, str]:
    manifest = load_manifest()
    target = resolve_target()
    assets = manifest.get("assets", {})
    if not isinstance(assets, dict) or target not in assets:
        raise RuntimeError(f"no release asset for target {target}")

    asset = assets[target]
    if not isinstance(asset, dict):
        raise RuntimeError(f"invalid manifest entry for target {target}")

    version_value = str(manifest["version"])
    install_dir = vera_home() / "bin" / version_value / target
    binary_path = install_dir / binary_name()
    if binary_path.exists():
        create_shim(binary_path)
        write_install_metadata(
            install_method=detect_wrapper_install_method(),
            version_value=version_value,
            binary_path=binary_path,
            target=target,
        )
        return binary_path, version_value

    with tempfile.TemporaryDirectory(prefix="vera-install-") as temp_dir_str:
        temp_dir = Path(temp_dir_str)
        archive_path = temp_dir / str(asset["archive"])
        extract_dir = temp_dir / "extract"

        print(f"Downloading Vera {version_value} for {target}...", file=sys.stderr)
        download_file(str(asset["download_url"]), archive_path)

        if sha256(archive_path) != str(asset["sha256"]):
            raise RuntimeError(f"checksum mismatch for {archive_path.name}")

        extract_archive(archive_path, extract_dir)
        extracted_binary = extract_dir / f"vera-{target}" / binary_name()
        install_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(extracted_binary, binary_path)
        if os.name != "nt":
            binary_path.chmod(binary_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    shim_path = create_shim(binary_path)
    if shim_path.parent.resolve() not in path_entries():
        print(
            f"Added Vera to {shim_path.parent}. Add that directory to PATH to run `vera` directly.",
            file=sys.stderr,
        )

    write_install_metadata(
        install_method=detect_wrapper_install_method(),
        version_value=version_value,
        binary_path=binary_path,
        target=target,
    )
    return binary_path, version_value


def run_binary(binary_path: Path, args: list[str]) -> int:
    result = subprocess.run([str(binary_path), *args], check=False)
    return result.returncode


def main() -> int:
    command = sys.argv[1] if len(sys.argv) > 1 else "help"
    rest = sys.argv[2:]
    binary_path, version_value = ensure_binary_installed()

    if command == "install":
        print(f"Vera {version_value} installed.", file=sys.stderr)
        return run_binary(binary_path, ["agent", "install", *rest])

    if command == "help":
        return run_binary(binary_path, ["--help"])

    return run_binary(binary_path, [command, *rest])


if __name__ == "__main__":
    raise SystemExit(main())
