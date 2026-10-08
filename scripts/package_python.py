"""Build and smoke-test the Python executable and native macOS Terminal app."""

import hashlib
import os
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def build(tag):
    if platform.system() not in {"Windows", "Darwin"}:
        raise SystemExit("Native Windows or macOS runner required")
    arch = "arm64" if platform.machine().lower() in {"arm64", "aarch64"} else "x86_64"
    name = f"cpass-python-{'windows' if sys.platform == 'win32' else 'macos'}-{arch}-{tag}"
    work = ROOT / "build" / "python-release"
    output = ROOT / "dist"
    package = work / name
    package.mkdir(parents=True, exist_ok=True)
    output.mkdir(exist_ok=True)
    subprocess.run(
        [
            sys.executable,
            "-m",
            "PyInstaller",
            "--noconfirm",
            "--onefile",
            "--console",
            "--name",
            "CxKitty",
            "--distpath",
            str(work / "binary"),
            "--workpath",
            str(work / "pyinstaller"),
            "--specpath",
            str(work),
            "--collect-data",
            "ddddocr",
            "--collect-binaries",
            "onnxruntime",
            "--add-data",
            f"{ROOT / 'config.yml'}:.",
            "--add-data",
            f"{ROOT / 'pyproject.toml'}:.",
            str(ROOT / "main.py"),
        ],
        cwd=ROOT,
        check=True,
    )
    binary_name = "CxKitty.exe" if sys.platform == "win32" else "CxKitty"
    resources = package
    if sys.platform == "darwin":
        app = package / "CxKitty.app"
        subprocess.run(
            [
                "osacompile",
                "-o",
                str(app),
                "-e",
                """on run
    set launcherPath to (POSIX path of (path to me)) & "Contents/Resources/launch.command"
    tell application "Terminal"
        activate
        do script (quoted form of launcherPath)
    end tell
end run""",
            ],
            check=True,
        )
        resources = app / "Contents" / "Resources"
        info_path = app / "Contents" / "Info.plist"
        with info_path.open("rb") as handle:
            info = plistlib.load(handle)
        version = tomllib.loads((ROOT / "pyproject.toml").read_text())["tool"]["poetry"]["version"]
        info.update(
            CFBundleIdentifier="io.github.hortus-edenensis.cpass",
            CFBundleShortVersionString=version,
        )
        with info_path.open("wb") as handle:
            plistlib.dump(info, handle)
        launcher = resources / "launch.command"
        launcher.write_text(
            """#!/bin/bash
set -e
bundle_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
data_dir="${CPASS_HOME:-$HOME/Library/Application Support/CxKitty}"
mkdir -p "$data_dir"
if [ ! -f "$data_dir/config.yml" ]; then
    cp "$bundle_dir/config.yml" "$data_dir/config.yml"
fi
cp "$bundle_dir/pyproject.toml" "$data_dir/pyproject.toml"
cd "$data_dir"
exec "$bundle_dir/CxKitty" "$@"
"""
        )
        launcher.chmod(0o755)
    shutil.copy2(work / "binary" / binary_name, resources / binary_name)
    for filename in ("config.yml", "pyproject.toml"):
        shutil.copy2(ROOT / filename, resources / filename)
    for filename in ("README.md", "LICENSE"):
        shutil.copy2(ROOT / filename, package / filename)
    shutil.copytree(ROOT / "docs", package / "docs", dirs_exist_ok=True)
    if sys.platform == "darwin":
        subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    with tempfile.TemporaryDirectory(prefix="cpass-frozen-smoke-") as temporary:
        smoke_dir = Path(temporary)
        if sys.platform == "win32":
            for filename in ("config.yml", "pyproject.toml"):
                shutil.copy2(resources / filename, smoke_dir / filename)
        executable = resources / ("launch.command" if sys.platform == "darwin" else binary_name)
        subprocess.run(
            [str(executable), "--self-check"],
            cwd=smoke_dir,
            env={**os.environ, "CPASS_HOME": str(smoke_dir)},
            check=True,
            timeout=180,
        )
    archive = output / f"{name}.zip"
    if sys.platform == "darwin":
        subprocess.run(
            ["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(package), str(archive)],
            check=True,
        )
    else:
        shutil.make_archive(str(archive.with_suffix("")), "zip", work, name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    print(f"Packaged and smoke-tested {archive.name}")


if __name__ == "__main__":
    build(sys.argv[1])
