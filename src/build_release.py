#!/usr/bin/env python3
from __future__ import annotations

import argparse
import fnmatch
import json
import platform
import shutil
import subprocess
import sys
import zipfile
from dataclasses import dataclass
from pathlib import Path

NOTICE_FILES = ("LICENSE", "THIRD_PARTY_NOTICES.txt", "about.txt")
SOURCE_EXCLUDED_DIR_NAMES = {
    ".git",
    ".my",
    ".idea",
    ".vscode",
    "target",
    "dist",
    "coverage",
    "criterion",
}
SOURCE_EXCLUDED_FILE_NAMES = {
    "tarpaulin-report.html",
    "cargo-tarpaulin-report.xml",
    "flamegraph.svg",
    ".DS_Store",
    "Thumbs.db",
    "Desktop.ini",
}
SOURCE_EXCLUDED_SUFFIXES = (
    ".rlib",
    ".rmeta",
    ".profraw",
    ".profdata",
    ".pdb",
    ".ilk",
    ".log",
    ".tmp",
    ".bak",
    ".swp",
    ".swo",
)


@dataclass(frozen=True)
class ProjectMetadata:
    package_name: str
    package_version: str
    target_dir: Path


def parse_args() -> tuple[argparse.Namespace, list[str]]:
    parser = argparse.ArgumentParser(
        description=(
            "Build the Rust project in release mode, copy license notices, "
            "and create source/binary release archives."
        )
    )
    parser.add_argument(
        "--target",
        help="Optional Rust target triple passed to cargo build.",
    )
    parser.add_argument(
        "--no-open",
        action="store_true",
        help="Do not open the release binary folder after building.",
    )
    parser.add_argument(
        "--no-package",
        action="store_true",
        help="Copy notice files but do not create source or binary zip archives.",
    )
    return parser.parse_known_args()


def command_text(command: list[str]) -> str:
    return " ".join(command)


def run_checked(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    print(f"$ {command_text(command)}")
    return subprocess.run(command, cwd=cwd, check=True)


def read_project_metadata(cargo: str, project_root: Path) -> ProjectMetadata:
    result = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps"],
        cwd=project_root,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(result.stdout)
    packages = metadata.get("packages")
    if not isinstance(packages, list) or not packages:
        raise ValueError("cargo metadata did not contain a root package")

    package = packages[0]
    package_name = package.get("name")
    package_version = package.get("version")
    if not isinstance(package_name, str) or not package_name:
        raise ValueError("cargo metadata did not contain a package name")
    if not isinstance(package_version, str) or not package_version:
        raise ValueError("cargo metadata did not contain a package version")

    target_directory = metadata.get("target_directory")
    target_dir = (
        Path(target_directory)
        if isinstance(target_directory, str) and target_directory
        else project_root / "target"
    )
    return ProjectMetadata(package_name, package_version, target_dir)


def release_binary_dir(target_dir: Path, target_triple: str | None) -> Path:
    if target_triple:
        return target_dir / target_triple / "release"
    return target_dir / "release"


def copy_notice_files(project_root: Path, binary_dir: Path) -> None:
    for name in NOTICE_FILES:
        source = project_root / name
        destination = binary_dir / name
        shutil.copy2(source, destination)
        print(f"copied notice file: {destination}")


def executable_file_name(package_name: str, target_triple: str | None) -> str:
    if target_triple:
        is_windows = "windows" in target_triple.lower()
    else:
        is_windows = platform.system() == "Windows"
    return f"{package_name}.exe" if is_windows else package_name


def package_platform_label(target_triple: str | None) -> str:
    if target_triple:
        return target_triple
    system = platform.system().lower() or "unknown"
    machine = platform.machine().lower() or "unknown"
    return f"{system}-{machine}"


def archive_base_name(package_name: str, package_version: str) -> str:
    return f"{package_name}-{package_version}"


def create_binary_archive(
    project_root: Path,
    binary_dir: Path,
    metadata: ProjectMetadata,
    target_triple: str | None,
) -> Path:
    binary_name = executable_file_name(metadata.package_name, target_triple)
    archive_name = (
        f"{archive_base_name(metadata.package_name, metadata.package_version)}-"
        f"{package_platform_label(target_triple)}-binary.zip"
    )
    archive_path = binary_dir / archive_name
    files = [(binary_dir / binary_name, binary_name)]
    files.extend((project_root / name, name) for name in NOTICE_FILES)
    write_zip_archive(archive_path, files)
    verify_archive_contains(archive_path, (binary_name, *NOTICE_FILES))
    return archive_path


def create_source_archive(
    project_root: Path,
    binary_dir: Path,
    metadata: ProjectMetadata,
) -> Path:
    archive_name = f"{archive_base_name(metadata.package_name, metadata.package_version)}-source.zip"
    archive_path = binary_dir / archive_name
    files = [
        (project_root / relative_path, relative_path.as_posix())
        for relative_path in source_distribution_files(project_root)
    ]
    write_zip_archive(archive_path, files)
    verify_archive_contains(archive_path, NOTICE_FILES)
    return archive_path


def write_zip_archive(archive_path: Path, files: list[tuple[Path, str]]) -> None:
    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for source, archive_name in files:
            if not source.is_file():
                raise FileNotFoundError(f"release archive input was not found: {source}")
            archive.write(source, archive_name)
    print(f"created release archive: {archive_path}")


def verify_archive_contains(archive_path: Path, required_names: tuple[str, ...]) -> None:
    with zipfile.ZipFile(archive_path) as archive:
        members = set(archive.namelist())
    missing = [name for name in required_names if name not in members]
    if missing:
        joined = ", ".join(missing)
        raise ValueError(f"{archive_path} is missing required release files: {joined}")
    joined = ", ".join(required_names)
    print(f"verified archive contents: {archive_path} contains {joined}")


def source_distribution_files(project_root: Path) -> list[Path]:
    git = shutil.which("git")
    if git:
        result = subprocess.run(
            [git, "ls-files", "-z"],
            cwd=project_root,
            check=True,
            capture_output=True,
            text=True,
        )
        relative_paths = [Path(path) for path in result.stdout.split("\0") if path]
    else:
        relative_paths = [
            path.relative_to(project_root)
            for path in project_root.rglob("*")
            if path.is_file()
        ]

    included_paths = [
        relative_path
        for relative_path in relative_paths
        if source_path_is_included(relative_path)
    ]
    return sorted(included_paths, key=lambda path: path.as_posix())


def source_path_is_included(relative_path: Path) -> bool:
    parts = relative_path.parts
    if any(part in SOURCE_EXCLUDED_DIR_NAMES for part in parts):
        return False

    file_name = relative_path.name
    if file_name in SOURCE_EXCLUDED_FILE_NAMES:
        return False
    if file_name.endswith("~"):
        return False
    if file_name.endswith(SOURCE_EXCLUDED_SUFFIXES):
        return False

    normalized = relative_path.as_posix()
    return not any(fnmatch.fnmatch(normalized, pattern) for pattern in ("*.log", "*.tmp", "*.bak"))


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
        metadata = read_project_metadata(cargo, project_root)
    except subprocess.CalledProcessError as error:
        return error.returncode
    except json.JSONDecodeError as error:
        print(f"error: failed to parse cargo metadata: {error}", file=sys.stderr)
        return 1
    except ValueError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    binary_dir = release_binary_dir(metadata.target_dir, args.target)
    print(f"release binary folder: {binary_dir}")

    if not binary_dir.is_dir():
        print("error: release binary folder was not created.", file=sys.stderr)
        return 1

    try:
        copy_notice_files(project_root, binary_dir)
        if not args.no_package:
            source_archive = create_source_archive(project_root, binary_dir, metadata)
            binary_archive = create_binary_archive(project_root, binary_dir, metadata, args.target)
            print(f"source archive: {source_archive}")
            print(f"binary archive: {binary_archive}")
    except OSError as error:
        print(f"error: failed to prepare release files: {error}", file=sys.stderr)
        return 1
    except (zipfile.BadZipFile, ValueError) as error:
        print(f"error: failed to verify release archives: {error}", file=sys.stderr)
        return 1

    if not args.no_open and not open_folder(binary_dir):
        print(
            "warning: no supported file manager command was found. "
            f"Open this folder manually: {binary_dir}",
            file=sys.stderr,
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
