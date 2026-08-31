#!/usr/bin/env python3
"""Install and verify local gate assets into a public repository.

The public project keeps gate assets ignored by default. This installer copies a
manifested, allowlisted closure from a local-only branch into the target repo and
records hashes so installation can be verified later.
"""

from __future__ import annotations
import argparse
import contextlib
import hashlib
import json
import os
import re
import secrets
import stat
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Dict


SCRIPT_NAME = "scripts/gate-assets.json"
RECORD_NAME = "suprnova-local-gate.json"
MANIFEST_SCHEMA = 1
INSTALL_RECORD_SCHEMA = 2
REQUIRED_BRANCH = "local/gate-infra"
REQUIRED_CANONICAL_ASSETS = frozenset(
    {
        SCRIPT_NAME,
        "scripts/gate.sh",
        "scripts/gate-runner.py",
        "scripts/gate-steps.json",
        "scripts/install-gate.py",
        ".githooks/pre-push",
        ".cargo/audit.toml",
    }
)
_OBJECT_ID = re.compile(r"^[0-9a-f]{40,64}$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")


@dataclass(frozen=True)
class InstallRecord:
    """Immutable metadata for an installed local gate state."""

    schema: int
    branch: str
    commit: str
    assets: Dict[str, str]
    capabilities: list[str]


def sha256_file(path: Path) -> str:
    """Return the SHA-256 digest of ``path`` as a lower-case hex string."""

    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(8192), b""):
            digest.update(chunk)
    return digest.hexdigest()

def _sha256_fd(fd: int) -> str:
    """Return the SHA-256 digest of an already-open regular file."""

    digest = hashlib.sha256()
    os.lseek(fd, 0, os.SEEK_SET)
    while chunk := os.read(fd, 65536):
        digest.update(chunk)
    return digest.hexdigest()


def _read_fd_bytes(fd: int) -> bytes:
    """Read all bytes from an already-open regular file."""

    chunks: list[bytes] = []
    os.lseek(fd, 0, os.SEEK_SET)
    while chunk := os.read(fd, 65536):
        chunks.append(chunk)
    return b"".join(chunks)


def _verify_fd_inside_root(
    fd: int,
    root: Path,
    *,
    label: str,
    relative: str,
) -> Path:
    """Resolve a pinned descriptor and require it to remain inside ``root``."""

    try:
        resolved = Path(f"/proc/self/fd/{fd}").resolve(strict=True)
        resolved.relative_to(root)
    except (OSError, RuntimeError, ValueError) as error:
        raise EnvironmentError(
            f"{label} descriptor resolves outside root: {relative}"
        ) from error
    return resolved


def _open_source_file(
    source_root: Path,
    source_path: Path,
    relative: str,
) -> tuple[int, os.stat_result]:
    """Open and pin a validated source without following its final component."""

    try:
        fd = os.open(
            source_path,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
        )
    except OSError as error:
        raise EnvironmentError(
            f"cannot open local gate source asset: {relative}: {error}"
        ) from error

    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode):
            raise EnvironmentError(
                f"local gate source asset is not a regular file: {relative}"
            )
        _verify_fd_inside_root(
            fd,
            source_root,
            label="local gate source",
            relative=relative,
        )
        return fd, metadata
    except BaseException:
        os.close(fd)
        raise


def _open_destination_parent(repo_root: Path, relative: str) -> int:
    """Create and pin an asset parent by walking from the pinned repo root."""

    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW
    try:
        current_fd = os.open(repo_root, flags)
    except OSError as error:
        raise EnvironmentError(
            f"cannot open destination repository root: {repo_root}: {error}"
        ) from error

    try:
        _verify_fd_inside_root(
            current_fd,
            repo_root,
            label="destination repository",
            relative=".",
        )
        for part in Path(relative).parent.parts:
            try:
                os.mkdir(part, mode=0o755, dir_fd=current_fd)
            except FileExistsError:
                pass
            except OSError as error:
                raise EnvironmentError(
                    f"cannot create destination repository parent for {relative}: {error}"
                ) from error

            try:
                next_fd = os.open(part, flags, dir_fd=current_fd)
            except OSError as error:
                raise EnvironmentError(
                    f"cannot open destination repository parent for {relative}: {error}"
                ) from error
            try:
                metadata = os.fstat(next_fd)
                if not stat.S_ISDIR(metadata.st_mode):
                    raise EnvironmentError(
                        f"destination repository parent is not a directory: {relative}"
                    )
                _verify_fd_inside_root(
                    next_fd,
                    repo_root,
                    label="destination repository",
                    relative=relative,
                )
            except BaseException:
                os.close(next_fd)
                raise

            os.close(current_fd)
            current_fd = next_fd
        return current_fd
    except BaseException:
        os.close(current_fd)
        raise


def _create_temporary_file(parent_fd: int, destination_name: str) -> tuple[str, int]:
    """Create an unpredictable exclusive temporary file in a pinned directory."""

    flags = (
        os.O_WRONLY
        | os.O_CREAT
        | os.O_EXCL
        | os.O_CLOEXEC
        | os.O_NOFOLLOW
    )
    for _attempt in range(128):
        temporary_name = (
            f".{destination_name}.install-{secrets.token_hex(16)}"
        )
        try:
            fd = os.open(temporary_name, flags, 0o600, dir_fd=parent_fd)
        except FileExistsError:
            continue
        except OSError as error:
            raise EnvironmentError(
                f"cannot create gate install temporary file for {destination_name}: {error}"
            ) from error

        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise EnvironmentError(
                    f"gate install temporary is not a regular file: {destination_name}"
                )
            return temporary_name, fd
        except BaseException:
            os.close(fd)
            try:
                os.unlink(temporary_name, dir_fd=parent_fd)
            except FileNotFoundError:
                pass
            raise

    raise EnvironmentError(
        f"cannot create unique gate install temporary file for {destination_name}"
    )


def _write_fd_bytes(destination_fd: int, contents: bytes) -> None:
    """Write immutable bytes to an already-open destination descriptor."""

    view = memoryview(contents)
    while view:
        written = os.write(destination_fd, view)
        if written == 0:
            raise OSError("short write while installing gate asset")
        view = view[written:]


def _stream_fd(source_fd: int, destination_fd: int) -> None:
    """Copy bytes between pinned descriptors, handling short writes."""

    os.lseek(source_fd, 0, os.SEEK_SET)
    while chunk := os.read(source_fd, 65536):
        _write_fd_bytes(destination_fd, chunk)


def _open_installed_file(
    parent_fd: int,
    destination_name: str,
    repo_root: Path,
    relative: str,
) -> tuple[int, os.stat_result]:
    """Open the replaced destination without following it and verify containment."""

    try:
        fd = os.open(
            destination_name,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
            dir_fd=parent_fd,
        )
    except OSError as error:
        raise EnvironmentError(
            f"cannot open installed gate destination: {relative}: {error}"
        ) from error

    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode):
            raise EnvironmentError(
                f"installed gate destination is not a regular file: {relative}"
            )
        _verify_fd_inside_root(
            fd,
            repo_root,
            label="installed gate destination",
            relative=relative,
        )
        return fd, metadata
    except BaseException:
        os.close(fd)
        raise


def _unlink_temporary(parent_fd: int, temporary_name: str) -> None:
    try:
        os.unlink(temporary_name, dir_fd=parent_fd)
    except FileNotFoundError:
        pass


def _install_asset(
    repo_root: Path,
    relative: str,
    source: int | bytes,
    source_mode: int,
) -> str:
    """Install one pinned descriptor or immutable snapshot atomically."""

    parent_fd = _open_destination_parent(repo_root, relative)
    destination_name = Path(relative).name
    with contextlib.ExitStack() as cleanup:
        cleanup.callback(os.close, parent_fd)
        try:
            existing = os.stat(
                destination_name,
                dir_fd=parent_fd,
                follow_symlinks=False,
            )
        except FileNotFoundError:
            existing = None
        except OSError as error:
            raise EnvironmentError(
                f"cannot inspect destination repository asset: {relative}: {error}"
            ) from error
        if existing is not None and stat.S_ISLNK(existing.st_mode):
            raise EnvironmentError(
                f"destination repository path contains symlink: {relative}"
            )

        temporary_name, temporary_fd = _create_temporary_file(
            parent_fd,
            destination_name,
        )
        cleanup.callback(_unlink_temporary, parent_fd, temporary_name)
        cleanup.callback(os.close, temporary_fd)
        _verify_fd_inside_root(
            temporary_fd,
            repo_root,
            label="gate install temporary",
            relative=relative,
        )
        if isinstance(source, int):
            _stream_fd(source, temporary_fd)
        else:
            _write_fd_bytes(temporary_fd, source)
        os.fsync(temporary_fd)
        os.fchmod(temporary_fd, source_mode)
        os.fsync(temporary_fd)
        temporary_metadata = os.fstat(temporary_fd)

        os.replace(
            temporary_name,
            destination_name,
            src_dir_fd=parent_fd,
            dst_dir_fd=parent_fd,
        )

        installed_fd, installed_metadata = _open_installed_file(
            parent_fd,
            destination_name,
            repo_root,
            relative,
        )
        cleanup.callback(os.close, installed_fd)
        if (
            installed_metadata.st_dev,
            installed_metadata.st_ino,
        ) != (
            temporary_metadata.st_dev,
            temporary_metadata.st_ino,
        ):
            raise EnvironmentError(
                f"installed gate destination changed during replacement: {relative}"
            )

        digest = _sha256_fd(installed_fd)
        try:
            current_metadata = os.stat(
                destination_name,
                dir_fd=parent_fd,
                follow_symlinks=False,
            )
        except OSError as error:
            raise EnvironmentError(
                f"cannot re-inspect installed gate destination: {relative}: {error}"
            ) from error
        if (
            stat.S_ISLNK(current_metadata.st_mode)
            or not stat.S_ISREG(current_metadata.st_mode)
            or (
                current_metadata.st_dev,
                current_metadata.st_ino,
            )
            != (
                installed_metadata.st_dev,
                installed_metadata.st_ino,
            )
        ):
            raise EnvironmentError(
                f"installed gate destination changed before hashing: {relative}"
            )
        os.fsync(parent_fd)
        return digest


def _validate_asset_path(relative: str) -> str:
    """Validate manifest asset path and reject traversal/absolute escapes."""

    if "/" in relative:
        normalized = Path(*relative.split("/"))
    else:
        normalized = Path(relative)

    if "\\" in relative:
        raise EnvironmentError(f"invalid gate asset path (backslash prohibited): {relative}")
    if normalized.is_absolute():
        raise EnvironmentError(f"invalid gate asset path (absolute disallowed): {relative}")
    if any(part == ".." for part in normalized.parts):
        raise EnvironmentError(f"invalid gate asset path (path traversal disallowed): {relative}")
    if relative.startswith(("/", "~")):
        raise EnvironmentError(f"invalid gate asset path: {relative}")

    return str(normalized)


def _validate_capabilities(raw: object, *, label: str) -> list[str]:
    if (
        not isinstance(raw, list)
        or not raw
        or any(type(capability) is not str or not capability for capability in raw)
        or len(set(raw)) != len(raw)
    ):
        raise EnvironmentError(f"invalid {label} capabilities")
    return list(raw)


def _validate_root(root: Path, *, label: str) -> Path:
    try:
        if root.is_symlink():
            raise EnvironmentError(f"{label} root must not be a symlink: {root}")
        resolved = root.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise EnvironmentError(f"invalid {label} root: {root}: {error}") from error
    if not resolved.is_dir():
        raise EnvironmentError(f"invalid {label} root: {root}")
    return resolved


def _validate_path_components(
    root: Path,
    relative: str,
    *,
    label: str,
    require_file: bool,
) -> Path:
    root_resolved = _validate_root(root, label=label)
    path = root
    missing = False
    for part in Path(relative).parts:
        path = path / part
        if missing:
            continue
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            missing = True
            continue
        except OSError as error:
            raise EnvironmentError(
                f"cannot inspect {label} path: {relative}: {error}"
            ) from error
        if stat.S_ISLNK(metadata.st_mode):
            raise EnvironmentError(f"{label} path contains symlink: {relative}")
        try:
            path.resolve(strict=True).relative_to(root_resolved)
        except (OSError, RuntimeError, ValueError) as error:
            raise EnvironmentError(
                f"{label} path resolves outside root: {relative}"
            ) from error

    if require_file:
        if missing:
            raise EnvironmentError(f"missing {label} asset: {relative}")
        try:
            metadata = path.lstat()
        except OSError as error:
            raise EnvironmentError(f"missing {label} asset: {relative}") from error
        if stat.S_ISLNK(metadata.st_mode):
            raise EnvironmentError(f"{label} path contains symlink: {relative}")
        if not stat.S_ISREG(metadata.st_mode):
            raise EnvironmentError(f"missing {label} asset: {relative}")
    return path


def _git_path(repo: Path, filename: str) -> Path:
    relative = subprocess.check_output(
        ["git", "rev-parse", "--git-path", filename],
        cwd=repo,
        text=True,
    ).strip()
    return repo / relative


def load_record(repo: Path) -> InstallRecord:
    """Load and validate the install record from a public repo worktree."""

    record_path = _git_path(repo, RECORD_NAME)
    try:
        payload = json.loads(record_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EnvironmentError(f"missing or invalid gate install record: {error}") from error
    if (
        not isinstance(payload, dict)
        or set(payload) != {"schema", "branch", "commit", "assets", "capabilities"}
        or type(payload["schema"]) is not int
        or payload["schema"] != INSTALL_RECORD_SCHEMA
        or payload["branch"] != REQUIRED_BRANCH
        or not isinstance(payload["commit"], str)
        or not _OBJECT_ID.fullmatch(payload["commit"])
        or not isinstance(payload["assets"], dict)
        or not payload["assets"]
    ):
        raise EnvironmentError("invalid gate install record")

    assets: Dict[str, str] = {}
    for raw_relative, digest in payload["assets"].items():
        if not isinstance(raw_relative, str) or not isinstance(digest, str):
            raise EnvironmentError("invalid gate install record assets")
        relative = _validate_asset_path(raw_relative)
        if not _SHA256.fullmatch(digest):
            raise EnvironmentError(f"invalid installed asset hash: {relative}")
        assets[relative] = digest
    missing = REQUIRED_CANONICAL_ASSETS - set(assets)
    if missing:
        raise EnvironmentError(
            "gate install record omits required gate assets: "
            + ", ".join(sorted(missing))
        )
    capabilities = _validate_capabilities(
        payload["capabilities"], label="gate install record"
    )
    return InstallRecord(
        schema=INSTALL_RECORD_SCHEMA,
        branch=REQUIRED_BRANCH,
        commit=payload["commit"],
        assets=assets,
        capabilities=capabilities,
    )


def _prepare_manifest_assets(source: Path, manifest: Dict[str, object]) -> list[tuple[str, Path]]:
    raw_assets = manifest.get("assets")
    if not isinstance(raw_assets, list):
        raise EnvironmentError("manifest assets must be a list")

    prepared: list[tuple[str, Path]] = []
    seen: set[str] = set()
    for raw_relative in raw_assets:
        if not isinstance(raw_relative, str):
            raise EnvironmentError("manifest asset entries must be strings")
        relative = _validate_asset_path(raw_relative)
        if relative in seen:
            raise EnvironmentError(f"duplicate gate manifest asset: {relative}")
        seen.add(relative)
        source_path = _validate_path_components(
            source,
            relative,
            label="local gate",
            require_file=True,
        )
        prepared.append((relative, source_path))

    missing = REQUIRED_CANONICAL_ASSETS - seen
    if missing:
        raise EnvironmentError(
            "manifest omits required gate assets: " + ", ".join(sorted(missing))
        )
    return prepared


def _validate_manifest(
    manifest: Dict[str, object], source: Path
) -> tuple[str, list[str], list[tuple[str, Path]]]:
    if not isinstance(manifest, dict):
        raise EnvironmentError("invalid gate manifest: expected JSON object")

    schema = manifest.get("schema")
    if type(schema) is not int or schema != MANIFEST_SCHEMA:
        raise EnvironmentError(f"invalid gate manifest schema: {schema!r}")

    branch = manifest.get("branch")
    if type(branch) is not str or branch != REQUIRED_BRANCH:
        raise EnvironmentError(f"invalid gate manifest branch: {branch!r}")

    capabilities = _validate_capabilities(
        manifest.get("capabilities"), label="gate manifest"
    )
    prepared = _prepare_manifest_assets(source, manifest)
    return branch, capabilities, prepared


def install(repo: Path, source: Path, commit: str) -> InstallRecord:
    """Copy ignored gate assets from ``source`` to ``repo`` and write a record."""

    if not _OBJECT_ID.fullmatch(commit):
        raise EnvironmentError(f"invalid local tooling commit: {commit!r}")
    repo_root = _validate_root(repo, label="destination repository")
    source_root = _validate_root(source, label="local gate source")
    manifest_path = _validate_path_components(
        source_root,
        SCRIPT_NAME,
        label="local gate source",
        require_file=True,
    )
    manifest_fd, manifest_metadata = _open_source_file(
        source_root,
        manifest_path,
        SCRIPT_NAME,
    )
    try:
        manifest_bytes = _read_fd_bytes(manifest_fd)
        manifest = json.loads(manifest_bytes.decode("utf-8"))
    finally:
        os.close(manifest_fd)
    metadata_branch, capabilities, prepared = _validate_manifest(
        manifest,
        source_root,
    )

    for relative, _source_path in prepared:
        _validate_path_components(
            repo_root,
            relative,
            label="destination repository",
            require_file=False,
        )

    hashes: Dict[str, str] = {}
    with contextlib.ExitStack() as source_stack:
        opened_sources: Dict[str, tuple[int | bytes, int]] = {
            SCRIPT_NAME: (
                manifest_bytes,
                stat.S_IMODE(manifest_metadata.st_mode),
            )
        }
        for relative, source_path in prepared:
            if relative == SCRIPT_NAME:
                continue
            source_fd, source_metadata = _open_source_file(
                source_root,
                source_path,
                relative,
            )
            source_stack.callback(os.close, source_fd)
            opened_sources[relative] = (
                source_fd,
                stat.S_IMODE(source_metadata.st_mode),
            )

        for relative, _source_path in prepared:
            source, source_mode = opened_sources[relative]
            hashes[relative] = _install_asset(
                repo_root,
                relative,
                source,
                source_mode,
            )

    subprocess.run(
        ["git", "config", "core.hooksPath", ".githooks"],
        cwd=repo_root,
        check=True,
    )

    record = InstallRecord(
        INSTALL_RECORD_SCHEMA,
        metadata_branch,
        commit,
        hashes,
        capabilities,
    )
    record_path = _git_path(repo_root, RECORD_NAME)
    record_path.write_text(
        json.dumps(asdict(record), indent=2) + "\n",
        encoding="utf-8",
    )
    return record


def _validate_record_against_manifest(
    repo: Path, record: InstallRecord
) -> list[tuple[str, Path]]:
    manifest_path = _validate_path_components(
        repo,
        SCRIPT_NAME,
        label="installed gate",
        require_file=True,
    )
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EnvironmentError(f"invalid installed gate manifest: {error}") from error
    branch, capabilities, prepared = _validate_manifest(manifest, repo)
    manifest_assets = [relative for relative, _path in prepared]
    if branch != record.branch:
        raise EnvironmentError("installed gate manifest branch differs from install record")
    if manifest_assets != list(record.assets):
        raise EnvironmentError("installed gate manifest assets differ from install record")
    if capabilities != record.capabilities:
        raise EnvironmentError(
            "installed gate manifest capabilities differ from install record"
        )
    return prepared


def _committed_file_is_executable(repo: Path, commit: str, relative: str) -> bool:
    """Return the executable bit recorded for one regular file in ``commit``."""

    try:
        encoded = subprocess.check_output(
            ["git", "ls-tree", "-z", "--full-tree", commit, "--", relative],
            cwd=repo,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError as error:
        raise EnvironmentError(
            f"local tooling commit tree is unavailable: {commit}"
        ) from error
    if not encoded:
        raise EnvironmentError(
            f"local tooling commit omits installed asset: {relative}"
        )

    entries = encoded.split(b"\0")
    entry, separator, committed_path = entries[0].partition(b"\t")
    fields = entry.split()
    try:
        object_id = fields[2].decode("ascii")
    except (IndexError, UnicodeDecodeError) as error:
        raise EnvironmentError(
            f"malformed local tooling commit entry: {relative}"
        ) from error
    if (
        len(entries) != 2
        or entries[1]
        or not separator
        or committed_path != os.fsencode(relative)
        or len(fields) != 3
        or fields[1] != b"blob"
        or not _OBJECT_ID.fullmatch(object_id)
    ):
        raise EnvironmentError(f"malformed local tooling commit entry: {relative}")
    if fields[0] not in {b"100644", b"100755"}:
        raise EnvironmentError(
            f"local tooling commit asset is not a regular file: {relative}"
        )
    return fields[0] == b"100755"


def verify_install(repo: Path) -> InstallRecord:
    """Verify installed asset bytes and modes, then return their install record."""

    record = load_record(repo)
    configured = subprocess.check_output(
        ["git", "config", "--get", "core.hooksPath"],
        cwd=repo,
        text=True,
    ).strip()

    if configured != ".githooks":
        raise EnvironmentError("core.hooksPath is not .githooks")

    prepared = _validate_record_against_manifest(repo, record)
    for relative, path in prepared:
        if sha256_file(path) != record.assets[relative]:
            raise EnvironmentError(f"installed gate asset drift: {relative}")
        expected_executable = _committed_file_is_executable(
            repo, record.commit, relative
        )
        installed_executable = bool(path.stat().st_mode & 0o111)
        if installed_executable != expected_executable:
            raise EnvironmentError(
                f"installed gate asset executable mode drift: {relative}"
            )

    return record


def _default_source_commit(source: Path) -> str:
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"],
        cwd=source,
        text=True,
    ).strip()


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Install local gate assets")
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--commit", default=None)
    parser.add_argument(
        "--verify-only",
        action="store_true",
        help="Verify existing installation and print JSON record only",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if not args.repo.is_dir():
        raise EnvironmentError(f"missing public repo: {args.repo}")

    if args.verify_only:
        record = verify_install(args.repo)
    else:
        if args.commit:
            commit = args.commit
        else:
            commit = _default_source_commit(args.source)
        record = install(args.repo, args.source, commit)

    print(json.dumps(asdict(record), indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
