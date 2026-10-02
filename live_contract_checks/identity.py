"""Identify checkout bytes for observer freshness, not package/catalog digests.

This conservative Git inventory includes tracked and non-ignored new files.
It is not a substitute for enumerating extra build inputs in the real collector.
"""

import hashlib
import json
from pathlib import Path
import stat
import subprocess

from live_contract_checks.acceptance import Incomplete, fields, need, relative_path


MAX_SOURCE_BYTES = 512 * 1024 * 1024


def checkout_identity(root: Path, paths: tuple[str, ...] = ()) -> str:
    need(root.is_dir(), "configured dependency checkout does not exist")
    root = root.resolve()
    try:
        listing = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *paths],
            capture_output=True, check=True, timeout=30,
        ).stdout
    except (OSError, subprocess.SubprocessError) as error:
        raise Incomplete("cannot inventory the configured Git checkout") from error
    names = set(listing.split(b"\0")) - {b""}
    for path in paths:
        relative_path(path, "explicit source")
        try:
            mode = (root / path).lstat().st_mode
        except FileNotFoundError:
            continue
        if not stat.S_ISDIR(mode):
            names.add(path.encode("utf-8", "surrogateescape"))
    names = sorted(names)
    need(bool(names), "checkout source inventory is empty")
    result = hashlib.sha256()
    total = 0
    for raw_name in names:
        name = raw_name.decode("utf-8", "surrogateescape")
        file = root / name
        result.update(raw_name + b"\0")
        try:
            metadata = file.lstat()
        except FileNotFoundError:
            result.update(b"deleted\0")
            continue
        need(stat.S_ISREG(metadata.st_mode) and file.resolve().is_relative_to(root),
             "checkout contains a link or special input; declare and capture its bytes explicitly")
        result.update(b"executable\0" if metadata.st_mode & 0o111 else b"regular\0")
        content = hashlib.sha256()
        with file.open("rb") as handle:
            while chunk := handle.read(65536):
                total += len(chunk)
                need(total <= MAX_SOURCE_BYTES, "checkout exceeds the observer's 512 MiB input limit")
                content.update(chunk)
        after = file.lstat()
        need((metadata.st_ino, metadata.st_size, metadata.st_mtime_ns, metadata.st_ctime_ns) ==
             (after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns),
             "checkout changed during input capture")
        result.update(content.digest())
    return "sha256:" + result.hexdigest()


def sdk_checkout(root: Path) -> Path:
    try:
        config = json.loads((root / "proof/sdk.json").read_text())
    except (OSError, ValueError) as error:
        raise Incomplete("configure proof/sdk.json for a SDK source checkout") from error
    fields(config, "schema checkout", "SDK configuration")
    need(type(config["schema"]) is int and config["schema"] == 1, "unsupported SDK configuration schema")
    location = config["checkout"]
    need(type(location) is str and bool(location), "no SDK source checkout is configured for observation")
    path = Path(location)
    if not path.is_absolute():
        path = root / path
    return path.resolve()


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    try:
        print(checkout_identity(sdk_checkout(root)))
    except (Incomplete, OSError) as error:
        print(json.dumps({"unverified": str(error)}))
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
