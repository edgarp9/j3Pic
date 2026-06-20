#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import platform
import shutil
import subprocess
import sys
from pathlib import Path


def parse_args() -> tuple[argparse.Namespace, list[str]]:
    parser = argparse.ArgumentParser(
        description="Build the Rust project in release mode and open the binary folder."
    )
    parser.add_argument(
        "--target",
        help="Optional Rust target triple passed to cargo build.",
    )
    return parser.parse_known_args()


def command_text(command: list[str]) -> str:
    return " ".join(command)


def run_checked(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    print(f"$ {command_text(command)}")
    return subprocess.run(command, cwd=cwd, check=True)


def read_target_dir(cargo: str, project_root: Path) -> Path:
    result = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps"],
        cwd=project_root,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(result.stdout)
    target_directory = metadata.get("target_directory")
    if not isinstance(target_directory, str) or not target_directory:
        return project_root / "target"
    return Path(target_directory)


def release_binary_dir(target_dir: Path, target_triple: str | None) -> Path:
    if target_triple:
        return target_dir / target_triple / "release"
    return target_dir / "release"


def open_folder(folder: Path) -> bool:
    system = platform.system()
    folder_text = str(folder)

    if system == "Windows":
        subprocess.Popen(["explorer", folder_text])
        return True

    if system == "Linux":
        opener = shutil.which("xdg-open")
        if opener:
            subprocess.Popen([opener, folder_text])
            return True

        gio = shutil.which("gio")
        if gio:
            subprocess.Popen([gio, "open", folder_text])
            return True

        return False

    if system == "Darwin":
        opener = shutil.which("open")
        if opener:
            subprocess.Popen([opener, folder_text])
            return True

    return False


def main() -> int:
    args, cargo_extra_args = parse_args()
    project_root = Path(__file__).resolve().parent

    cargo = shutil.which("cargo")
    if not cargo:
        print("error: cargo was not found in PATH.", file=sys.stderr)
        return 1

    cargo_command = [cargo, "build", "--release"]
    if args.target:
        cargo_command.extend(["--target", args.target])
    cargo_command.extend(cargo_extra_args)

    try:
        run_checked(cargo_command, project_root)
        target_dir = read_target_dir(cargo, project_root)
    except subprocess.CalledProcessError as error:
        return error.returncode
    except json.JSONDecodeError as error:
        print(f"error: failed to parse cargo metadata: {error}", file=sys.stderr)
        return 1

    binary_dir = release_binary_dir(target_dir, args.target)
    print(f"release binary folder: {binary_dir}")

    if not binary_dir.is_dir():
        print("error: release binary folder was not created.", file=sys.stderr)
        return 1

    if not open_folder(binary_dir):
        print(
            "warning: no supported file manager command was found. "
            f"Open this folder manually: {binary_dir}",
            file=sys.stderr,
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
