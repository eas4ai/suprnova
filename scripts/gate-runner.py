#!/usr/bin/env python3
"""Classified, process-safe local gate runner."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import secrets
import shutil
import signal
import stat
import subprocess
import sys
import threading
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from enum import Enum
from pathlib import Path
from typing import Mapping


class Outcome(str, Enum):
    """Terminal classification for a gate step."""

    PASS = "pass"
    FAIL = "fail"
    TIMEOUT = "timeout"
    ENVIRONMENT = "environment"
    INTERRUPTED = "interrupted"
    LEAK_DETECTED = "leak-detected"



class DeltaClass(str, Enum):
    """Fail-closed classification of a complete tree-to-tree net delta."""

    EMPTY = "empty"
    DOCS_ONLY = "docs-only"
    CODE = "code"
    MIXED = "mixed"

@dataclass(frozen=True)
class Step:
    """One immutable gate registry entry."""

    id: str
    name: str
    tiers: tuple[str, ...]
    argv: tuple[str, ...]
    timeout_seconds: int
    category: str
    capabilities: tuple[str, ...]


@dataclass
class RunContext:
    """Per-run resources shared by step executions."""

    repo: Path
    run_id: str
    run_dir: Path
    tier: str
    env: Mapping[str, str]
    termination_grace_seconds: float = 2.0
    interrupt_event: threading.Event | None = None
    container_cli: tuple[str, ...] | None = ("docker",)

    def __post_init__(self) -> None:
        if self.interrupt_event is None:
            self.interrupt_event = threading.Event()


@dataclass(frozen=True)
class StepResult:
    """One classified step result written to the JSONL ledger."""

    step: str
    tier: str
    outcome: Outcome
    seconds: float
    exit_code: int | None
    argv: tuple[str, ...]
    log_path: str
    started: bool
    leaks: tuple[dict[str, object], ...] = ()
    message: str | None = None

    def to_json(self) -> dict[str, object]:
        payload = asdict(self)
        payload["outcome"] = self.outcome.value
        payload["argv"] = list(self.argv)
        payload["leaks"] = list(self.leaks)
        return payload


_SAFE_STEP_ID = re.compile(r"^[a-z0-9][a-z0-9-]*$")
_RUN_ENV_PREFIX = b"SUPRNOVA_GATE_RUN_ID="
_URL_USERINFO_SECRET = re.compile(
    r"(?i)([a-z][a-z0-9+.-]*://[^\s/:@]+:)([^@\s/]+)(?=@)"
)
_PASSWORD_LIKE_SECRET = re.compile(
    r"""(?ix)
    (
        \b[a-z0-9_]*(?:password|passwd|pwd)\b
        ["']?\s*(?:=|:)\s*["']?
    )
    ([^\s,;"']+)
    """
)


def _missing_capabilities(step: Step, env: Mapping[str, str]) -> list[str]:
    path = env.get("PATH")
    return [
        capability
        for capability in step.capabilities
        if shutil.which(capability, path=path) is None
    ]


def _proc_stat(pid: int) -> tuple[int, int] | None:
    try:
        raw = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    except (FileNotFoundError, PermissionError, ProcessLookupError):
        return None
    marker = raw.rfind(") ")
    if marker < 0:
        return None
    fields = raw[marker + 2 :].split()
    if len(fields) < 3:
        return None
    return int(fields[1]), int(fields[2])


def _group_members(process_group: int) -> list[int]:
    members: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        stat = _proc_stat(int(entry.name))
        if stat is not None and stat[1] == process_group:
            members.append(int(entry.name))
    return members


def _wait_for_group_exit(process_group: int, seconds: float) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if not _group_members(process_group):
            return True
        time.sleep(0.02)
    return not _group_members(process_group)


def _terminate_group(process_group: int, grace_seconds: float) -> None:
    if not _group_members(process_group):
        return
    try:
        os.killpg(process_group, signal.SIGTERM)
    except ProcessLookupError:
        return
    if _wait_for_group_exit(process_group, grace_seconds):
        return
    try:
        os.killpg(process_group, signal.SIGKILL)
    except ProcessLookupError:
        return
    _wait_for_group_exit(process_group, grace_seconds)


def _scan_run_processes(run_id: str) -> list[dict[str, object]]:
    expected = _RUN_ENV_PREFIX + run_id.encode()
    found: list[dict[str, object]] = []
    current_pid = os.getpid()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        if pid == current_pid:
            continue
        try:
            environ = (entry / "environ").read_bytes().split(b"\0")
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if expected not in environ:
            continue
        stat = _proc_stat(pid)
        try:
            command = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(
                errors="replace"
            ).strip()
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            command = ""
        found.append(
            {
                "kind": "process",
                "pid": pid,
                "process_group": stat[1] if stat is not None else None,
                "command": command,
            }
        )
    return found


def _terminate_leaked_processes(
    leaks: list[dict[str, object]], run_id: str, grace_seconds: float
) -> None:
    for leak in leaks:
        pid = int(leak["pid"])
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + grace_seconds
    while time.monotonic() < deadline and _scan_run_processes(run_id):
        time.sleep(0.02)
    for leak in _scan_run_processes(run_id):
        try:
            os.kill(int(leak["pid"]), signal.SIGKILL)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + grace_seconds
    while time.monotonic() < deadline and _scan_run_processes(run_id):
        time.sleep(0.02)


def _cleanup_containers(context: RunContext) -> tuple[list[dict[str, object]], str | None]:
    if context.container_cli is None:
        return [], None
    command = [
        *context.container_cli,
        "ps",
        "-a",
        "--filter",
        f"label=suprnova-gate-run={context.run_id}",
        "--format",
        "{{.ID}}\t{{.Image}}",
    ]
    try:
        listed = subprocess.run(
            command,
            cwd=context.repo,
            env=dict(context.env),
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return [], f"container cleanup scan failed: {error}"
    if listed.returncode != 0:
        detail = listed.stderr.strip() or f"exit {listed.returncode}"
        return [], f"container cleanup scan failed: {detail}"
    containers: list[dict[str, object]] = []
    for line in listed.stdout.splitlines():
        if not line.strip():
            continue
        container_id, separator, image = line.partition("\t")
        if not separator or not container_id:
            return containers, f"invalid container scan output: {line!r}"
        containers.append(
            {"kind": "container", "id": container_id.strip(), "image": image.strip()}
        )
    if containers:
        removed = subprocess.run(
            [
                *context.container_cli,
                "rm",
                "-f",
                *[str(container["id"]) for container in containers],
            ],
            cwd=context.repo,
            env=dict(context.env),
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        if removed.returncode != 0:
            detail = removed.stderr.strip() or f"exit {removed.returncode}"
            return containers, f"container cleanup failed: {detail}"
    return containers, None


def _append_result(run_dir: Path, result: StepResult) -> None:
    ledger = run_dir / "results.jsonl"
    with ledger.open("a", encoding="utf-8") as handle:
        json.dump(result.to_json(), handle, sort_keys=True, separators=(",", ":"))
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())


def _environment_result(
    step: Step, context: RunContext, message: str, log_path: Path
) -> StepResult:
    return StepResult(
        step=step.id,
        tier=context.tier,
        outcome=Outcome.ENVIRONMENT,
        seconds=0.0,
        exit_code=None,
        argv=step.argv,
        log_path=str(log_path),
        started=False,
        message=message,
    )


def run_step(step: Step, context: RunContext) -> StepResult:
    """Execute one step in a dedicated process group and classify its outcome."""

    if not _SAFE_STEP_ID.fullmatch(step.id):
        return _environment_result(
            step, context, f"invalid step id: {step.id!r}", context.run_dir / "invalid.log"
        )
    context.run_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    context.run_dir.chmod(0o700)
    log_path = context.run_dir / f"{step.id}.log"
    missing = _missing_capabilities(step, context.env)
    if missing:
        return _environment_result(
            step,
            context,
            f"missing required capability: {', '.join(missing)}",
            log_path,
        )

    child_env = dict(context.env)
    child_env["SUPRNOVA_GATE_RUN_ID"] = context.run_id
    started_at = time.monotonic()
    try:
        log = log_path.open("wb")
    except OSError as error:
        return _environment_result(step, context, f"cannot open step log: {error}", log_path)
    try:
        log_path.chmod(0o600)
    except OSError as error:
        log.close()
        return _environment_result(step, context, f"cannot secure step log: {error}", log_path)

    process: subprocess.Popen[bytes] | None = None
    outcome = Outcome.FAIL
    exit_code: int | None = None
    message: str | None = None
    leaks: list[dict[str, object]] = []
    try:
        try:
            process = subprocess.Popen(
                step.argv,
                cwd=context.repo,
                env=child_env,
                start_new_session=True,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        except OSError as error:
            log.close()
            return _environment_result(
                step, context, f"cannot start step: {error}", log_path
            )

        deadline = started_at + step.timeout_seconds
        while True:
            exit_code = process.poll()
            if exit_code is not None:
                outcome = Outcome.PASS if exit_code == 0 else Outcome.FAIL
                break
            if context.interrupt_event is not None and context.interrupt_event.is_set():
                outcome = Outcome.INTERRUPTED
                message = "gate interrupted"
                break
            if time.monotonic() >= deadline:
                outcome = Outcome.TIMEOUT
                message = f"step exceeded {step.timeout_seconds}s timeout"
                break
            time.sleep(0.02)

        _terminate_group(process.pid, context.termination_grace_seconds)
        try:
            waited = process.wait(timeout=context.termination_grace_seconds)
            if exit_code is None and outcome not in (Outcome.TIMEOUT, Outcome.INTERRUPTED):
                exit_code = waited
        except subprocess.TimeoutExpired:
            _terminate_group(process.pid, context.termination_grace_seconds)
            process.wait(timeout=context.termination_grace_seconds)
    finally:
        log.close()

    process_leaks = _scan_run_processes(context.run_id)
    if process_leaks:
        leaks.extend(process_leaks)
        _terminate_leaked_processes(
            process_leaks, context.run_id, context.termination_grace_seconds
        )
    container_leaks, cleanup_error = _cleanup_containers(context)
    leaks.extend(container_leaks)
    if cleanup_error is not None:
        message = cleanup_error
        if outcome not in (Outcome.TIMEOUT, Outcome.INTERRUPTED):
            outcome = Outcome.ENVIRONMENT
    elif leaks and outcome not in (Outcome.TIMEOUT, Outcome.INTERRUPTED):
        outcome = Outcome.LEAK_DETECTED
        message = "run-owned resources escaped normal cleanup"

    result = StepResult(
        step=step.id,
        tier=context.tier,
        outcome=outcome,
        seconds=round(time.monotonic() - started_at, 6),
        exit_code=exit_code,
        argv=step.argv,
        log_path=str(log_path),
        started=True,
        leaks=tuple(leaks),
        message=message,
    )
    _append_result(context.run_dir, result)
    return result


@dataclass(frozen=True)
class GateStamp:
    """Schema-2 proof that one exact tree and gate definition passed."""

    schema: int
    tier: str
    tree: str
    commit: str
    toolchain: str
    steps_hash: str
    finished_at: str
    run_id: str
    code_provenance: str | None
    local_tooling_commit: str


@dataclass(frozen=True)
class StampValidation:
    """A stamp authorization decision with an actionable reason."""

    valid: bool
    message: str


@dataclass(frozen=True)
class PushAuthorization:
    """Authorization decision for a complete pre-push stdin batch."""

    allowed: bool
    message: str


@dataclass(frozen=True)
class InheritanceDecision:
    """Validated documentation inheritance from one original code tree."""

    valid: bool
    delta_class: DeltaClass
    code_provenance: str | None
    message: str


@dataclass(frozen=True)
class GatePlan:
    """Exact ordered step selection and stamp provenance for one run."""

    steps: tuple[Step, ...]
    delta_class: DeltaClass
    inherited: bool
    code_provenance: str | None
    message: str


@dataclass(frozen=True)
class InstallRecord:
    """Installed local gate closure used for provenance and hashing."""

    schema: int
    branch: str
    commit: str
    assets: dict[str, str]
    capabilities: tuple[str, ...]


_STAMP_KEYS = {
    "schema",
    "tier",
    "tree",
    "commit",
    "toolchain",
    "steps_hash",
    "finished_at",
    "run_id",
    "code_provenance",
    "local_tooling_commit",
}
_OBJECT_ID = re.compile(r"^[0-9a-f]{40,64}$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_RELEASE_REF = re.compile(r"^refs/tags/v[0-9]")
_ZERO_OBJECT = re.compile(r"^0+$")
_LOCAL_GATE_ASSET_ROOTS = frozenset({"scripts", ".githooks", ".cargo"})
_INSTALL_RECORD_SCHEMA = 2
_GATE_ASSET_MANIFEST = "scripts/gate-assets.json"
_REQUIRED_CANONICAL_ASSETS = frozenset(
    {
        _GATE_ASSET_MANIFEST,
        "scripts/gate.sh",
        "scripts/gate-runner.py",
        "scripts/gate-steps.json",
        "scripts/install-gate.py",
        ".githooks/pre-push",
        ".cargo/audit.toml",
    }
)



def _git_output(
    repo: Path, argv: list[str], *, env: Mapping[str, str] | None = None
) -> str:
    return subprocess.check_output(
        ["git", *argv],
        cwd=repo,
        env=None if env is None else dict(env),
        text=True,
        stderr=subprocess.DEVNULL,
    ).strip()


def _git_path(repo: Path, name: str) -> Path:
    path = Path(_git_output(repo, ["rev-parse", "--git-path", name]))
    return path if path.is_absolute() else repo / path


def _clean_tree(repo: Path) -> bool:
    return not _git_output(repo, ["status", "--porcelain"])


def _validate_local_path(relative: str) -> None:
    path = Path(relative)
    if (
        not relative
        or "\\" in relative
        or path.is_absolute()
        or relative.startswith(("~", "/"))
        or ".." in path.parts
    ):
        raise EnvironmentError(f"invalid local gate asset path: {relative!r}")


def _validated_repo_file(repo: Path, relative: str, *, label: str) -> Path:
    _validate_local_path(relative)
    try:
        if repo.is_symlink():
            raise EnvironmentError(f"repository root must not be a symlink: {repo}")
        repo_root = repo.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise EnvironmentError(f"cannot resolve repository root: {error}") from error

    path = repo
    for part in Path(relative).parts:
        path = path / part
        try:
            metadata = path.lstat()
        except OSError as error:
            raise EnvironmentError(f"missing {label}: {relative}") from error
        if stat.S_ISLNK(metadata.st_mode):
            raise EnvironmentError(f"{label} path contains symlink: {relative}")
        try:
            path.resolve(strict=True).relative_to(repo_root)
        except (OSError, RuntimeError, ValueError) as error:
            raise EnvironmentError(
                f"{label} resolves outside repository: {relative}"
            ) from error
    if not stat.S_ISREG(path.lstat().st_mode):
        raise EnvironmentError(f"missing {label}: {relative}")
    return path


def _validate_capabilities(raw: object, *, label: str) -> tuple[str, ...]:
    if (
        not isinstance(raw, list)
        or not raw
        or any(type(capability) is not str or not capability for capability in raw)
        or len(set(raw)) != len(raw)
    ):
        raise EnvironmentError(f"invalid {label} capabilities")
    return tuple(raw)


def _manifest_trust_closure(payload: object) -> tuple[tuple[str, ...], tuple[str, ...]]:
    if (
        not isinstance(payload, dict)
        or set(payload) != {"schema", "branch", "assets", "capabilities"}
        or type(payload["schema"]) is not int
        or payload["schema"] != 1
        or payload["branch"] != "local/gate-infra"
        or not isinstance(payload["assets"], list)
        or not payload["assets"]
    ):
        raise EnvironmentError("invalid installed gate asset manifest")
    assets: list[str] = []
    for relative in payload["assets"]:
        if not isinstance(relative, str):
            raise EnvironmentError("invalid installed gate manifest assets")
        _validate_local_path(relative)
        if relative in assets:
            raise EnvironmentError(f"duplicate installed gate manifest asset: {relative}")
        assets.append(relative)
    missing = _REQUIRED_CANONICAL_ASSETS - set(assets)
    if missing:
        raise EnvironmentError(
            "installed gate manifest omits required gate assets: "
            + ", ".join(sorted(missing))
        )
    capabilities = _validate_capabilities(
        payload["capabilities"], label="installed gate manifest"
    )
    return tuple(assets), capabilities


def _validate_record_manifest(repo: Path, record: InstallRecord) -> bytes:
    path = _validated_repo_file(
        repo, _GATE_ASSET_MANIFEST, label="installed gate manifest"
    )
    data = path.read_bytes()
    try:
        payload = json.loads(data)
    except json.JSONDecodeError as error:
        raise EnvironmentError(f"invalid installed gate asset manifest: {error}") from error
    assets, capabilities = _manifest_trust_closure(payload)
    if assets != tuple(record.assets):
        raise EnvironmentError(
            "installed gate manifest assets differ from install record"
        )
    if capabilities != record.capabilities:
        raise EnvironmentError(
            "installed gate manifest capabilities differ from install record"
        )
    return data

def _validate_step_executable_path(executable: str) -> None:
    if (
        "/" not in executable
        and "\\" not in executable
        and not executable.startswith("~")
    ):
        return
    path = Path(executable)
    if (
        "\\" in executable
        or path.is_absolute()
        or executable.startswith(("~", "/"))
        or ".." in path.parts
    ):
        raise EnvironmentError(
            f"invalid gate step executable path: {executable!r}"
        )

def _repository_local_gate_argument(repo_root: Path, argument: str) -> str | None:
    candidates = [argument]
    if "=" in argument:
        candidates.append(argument.split("=", 1)[1])
    for candidate in candidates:
        normalized = candidate.replace("\\", "/")
        path = Path(normalized)
        if path.is_absolute():
            try:
                relative = path.relative_to(repo_root)
            except ValueError:
                continue
            if relative.parts and relative.parts[0] in _LOCAL_GATE_ASSET_ROOTS:
                return candidate
            continue

        raw_parts = tuple(normalized.split("/"))
        collapsed_parts = tuple(os.path.normpath(normalized).split("/"))
        for parts in (raw_parts, collapsed_parts):
            first = next(
                (part for part in parts if part not in {"", ".", ".."}),
                None,
            )
            if first in _LOCAL_GATE_ASSET_ROOTS:
                return candidate
    return None


def _preflight_installed_asset(
    repo: Path,
    repo_root: Path,
    install: InstallRecord,
    relative: str,
    *,
    executable: bool,
) -> str | None:
    try:
        if executable:
            _validate_step_executable_path(relative)
        else:
            _validate_local_path(relative)
    except EnvironmentError as error:
        return str(error)
    label = "gate helper" if executable else "gate asset argument"
    if relative not in install.assets:
        return f"{label} is outside the installed manifest closure: {relative}"
    try:
        path = _validated_repo_file(repo, relative, label=label)
    except EnvironmentError as error:
        return str(error)
    try:
        path.resolve(strict=True).relative_to(repo_root)
    except (OSError, RuntimeError, ValueError):
        return f"{label} resolves outside repository: {relative}"
    required_mode = os.X_OK if executable else os.R_OK
    if not os.access(path, required_mode):
        requirement = "executable" if executable else "readable"
        return f"{label} is not {requirement}: {relative}"
    return None



def load_install_record(repo: Path) -> InstallRecord:
    """Load the installer-owned closure record from Git metadata."""

    path = _git_path(repo, "suprnova-local-gate.json")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EnvironmentError(f"missing or invalid local gate install record: {error}")
    if (
        not isinstance(payload, dict)
        or set(payload) != {"schema", "branch", "commit", "assets", "capabilities"}
        or type(payload["schema"]) is not int
        or payload["schema"] != _INSTALL_RECORD_SCHEMA
        or payload["branch"] != "local/gate-infra"
        or not isinstance(payload["commit"], str)
        or not _OBJECT_ID.fullmatch(payload["commit"])
        or not isinstance(payload["assets"], dict)
        or not payload["assets"]
    ):
        raise EnvironmentError("invalid local gate install record")
    assets: dict[str, str] = {}
    for relative, digest in payload["assets"].items():
        if not isinstance(relative, str) or not isinstance(digest, str):
            raise EnvironmentError("invalid local gate install record assets")
        _validate_local_path(relative)
        if not _SHA256.fullmatch(digest):
            raise EnvironmentError(f"invalid installed asset hash: {relative}")
        assets[relative] = digest
    missing = _REQUIRED_CANONICAL_ASSETS - set(assets)
    if missing:
        raise EnvironmentError(
            "local gate install record omits required gate assets: "
            + ", ".join(sorted(missing))
        )
    capabilities = _validate_capabilities(
        payload["capabilities"], label="local gate install record"
    )
    return InstallRecord(
        schema=_INSTALL_RECORD_SCHEMA,
        branch="local/gate-infra",
        commit=payload["commit"],
        assets=assets,
        capabilities=capabilities,
    )


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


def verify_local_install(repo: Path) -> InstallRecord:
    """Verify hooks plus source-commit byte and executable-mode provenance."""

    record = load_install_record(repo)
    try:
        hooks_path = _git_output(repo, ["config", "--get", "core.hooksPath"])
    except subprocess.CalledProcessError as error:
        raise EnvironmentError("core.hooksPath is not .githooks") from error
    if hooks_path != ".githooks":
        raise EnvironmentError("core.hooksPath is not .githooks")
    manifest_data = _validate_record_manifest(repo, record)
    try:
        subprocess.run(
            ["git", "cat-file", "-e", f"{record.commit}^{{commit}}"],
            cwd=repo,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError as error:
        raise EnvironmentError(
            f"local tooling commit is unavailable: {record.commit}"
        ) from error
    for relative, expected in record.assets.items():
        path = _validated_repo_file(repo, relative, label="local gate tooling")
        installed = path.read_bytes()
        digest = hashlib.sha256(installed).hexdigest()
        if digest != expected:
            raise EnvironmentError(f"installed local gate tooling drift: {relative}")
        try:
            committed = subprocess.check_output(
                ["git", "show", f"{record.commit}:{relative}"],
                cwd=repo,
                stderr=subprocess.DEVNULL,
            )
        except subprocess.CalledProcessError as error:
            raise EnvironmentError(
                f"local tooling commit omits installed asset: {relative}"
            ) from error
        if installed != committed:
            raise EnvironmentError(
                f"installed local gate tooling differs from local tooling commit: {relative}"
            )
        expected_executable = _committed_file_is_executable(
            repo, record.commit, relative
        )
        installed_executable = bool(path.stat().st_mode & 0o111)
        if installed_executable != expected_executable:
            raise EnvironmentError(
                f"installed local gate tooling executable mode drift: {relative}"
            )
        if relative == _GATE_ASSET_MANIFEST and committed != manifest_data:
            raise EnvironmentError(
                "committed gate manifest differs from installed gate manifest"
            )
    return record


def _hash_record(digest: object, domain: bytes, relative: str, data: bytes) -> None:
    encoded = relative.encode()
    digest.update(domain)
    digest.update(len(encoded).to_bytes(4, "big"))
    digest.update(encoded)
    digest.update(len(data).to_bytes(8, "big"))
    digest.update(data)


def _tracked_gate_inputs(repo: Path) -> list[str]:
    output = subprocess.check_output(
        ["git", "ls-files", "-z"],
        cwd=repo,
    )
    paths = [raw.decode() for raw in output.split(b"\0") if raw]
    return sorted(
        relative
        for relative in paths
        if Path(relative).name in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
        or relative == ".config/nextest.toml"
    )


def compute_steps_hash(repo: Path, tier: str) -> str:
    """Hash current installed tooling, capabilities, and tracked gate inputs."""

    if tier not in {"default", "full"}:
        raise EnvironmentError(f"invalid stamp tier: {tier!r}")
    record = load_install_record(repo)
    _validate_record_manifest(repo, record)
    digest = hashlib.sha256()
    digest.update(b"suprnova-gate-steps-v2\0")
    digest.update(tier.encode() + b"\0")
    capability_bytes = b"\0".join(
        capability.encode("utf-8") for capability in record.capabilities
    )
    _hash_record(digest, b"C", "capabilities", capability_bytes)
    for relative in record.assets:
        path = _validated_repo_file(repo, relative, label="local gate tooling")
        _hash_record(digest, b"L", relative, path.read_bytes())
    for relative in _tracked_gate_inputs(repo):
        try:
            data = (repo / relative).read_bytes()
        except OSError as error:
            raise EnvironmentError(f"missing tracked gate input: {relative}") from error
        _hash_record(digest, b"P", relative, data)
    return digest.hexdigest()


def current_toolchain(
    repo: Path, env: Mapping[str, str] | None = None
) -> str:
    """Return the exact rustc version used to authorize a stamp."""

    try:
        return subprocess.check_output(
            ["rustc", "--version"],
            cwd=repo,
            env=None if env is None else dict(env),
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError) as error:
        raise EnvironmentError(f"cannot identify rustc toolchain: {error}") from error


def _stamp_from_payload(payload: object) -> GateStamp | None:
    if not isinstance(payload, dict) or set(payload) != _STAMP_KEYS:
        return None
    if (
        type(payload["schema"]) is not int
        or payload["schema"] != 2
        or payload["tier"] not in {"default", "full"}
        or not isinstance(payload["tree"], str)
        or not _OBJECT_ID.fullmatch(payload["tree"])
        or not isinstance(payload["commit"], str)
        or not _OBJECT_ID.fullmatch(payload["commit"])
        or not isinstance(payload["toolchain"], str)
        or not payload["toolchain"]
        or not isinstance(payload["steps_hash"], str)
        or not _SHA256.fullmatch(payload["steps_hash"])
        or not isinstance(payload["finished_at"], str)
        or not payload["finished_at"]
        or not isinstance(payload["run_id"], str)
        or not payload["run_id"]
        or (
            payload["code_provenance"] is not None
            and (
                not isinstance(payload["code_provenance"], str)
                or not _OBJECT_ID.fullmatch(payload["code_provenance"])
            )
        )
        or not isinstance(payload["local_tooling_commit"], str)
        or not _OBJECT_ID.fullmatch(payload["local_tooling_commit"])
    ):
        return None
    return GateStamp(**payload)


def load_stamp(repo: Path) -> GateStamp | None:
    """Load a structurally valid schema-2 stamp; legacy content is invalid."""

    path = _git_path(repo, "suprnova-gate-pass")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return _stamp_from_payload(payload)


def write_stamp(repo: Path, stamp: GateStamp) -> None:
    """Atomically write one exact schema-2 stamp."""

    if _stamp_from_payload(asdict(stamp)) is None:
        raise ValueError("refusing to write invalid schema-2 gate stamp")
    path = _git_path(repo, "suprnova-gate-pass")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.tmp-{os.getpid()}")
    temporary.write_text(
        json.dumps(asdict(stamp), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def build_stamp(
    repo: Path,
    *,
    tier: str,
    run_id: str,
    code_provenance: str | None,
    env: Mapping[str, str] | None = None,
) -> GateStamp:
    """Build a current-tree stamp only after clean install and tree checks."""

    if not _clean_tree(repo):
        raise EnvironmentError("public working tree is dirty; no stamp written")
    record = verify_local_install(repo)
    commit = _git_output(repo, ["rev-parse", "HEAD"])
    tree = _git_output(repo, ["rev-parse", "HEAD^{tree}"])
    return GateStamp(
        schema=2,
        tier=tier,
        tree=tree,
        commit=commit,
        toolchain=current_toolchain(repo, env),
        steps_hash=compute_steps_hash(repo, tier),
        finished_at=datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        run_id=run_id,
        code_provenance=code_provenance,
        local_tooling_commit=record.commit,
    )


def resolve_code_provenance(stamp: GateStamp) -> str:
    """Return the original code-tested tree without trusting stamp chains."""

    return stamp.code_provenance or stamp.tree


def validate_stamp(
    repo: Path,
    stamp: GateStamp | None,
    *,
    required_tier: str = "default",
    pushed_tree: str | None = None,
    pushed_commit: str | None = None,
    release: bool = False,
    env: Mapping[str, str] | None = None,
) -> StampValidation:
    """Validate tree, tier, toolchain, definition, cleanliness, and provenance."""

    if stamp is None:
        return StampValidation(False, "missing or invalid schema-2 gate stamp")
    if stamp.tier not in {"default", "full"}:
        return StampValidation(False, f"invalid stamp tier: {stamp.tier!r}")
    if required_tier not in {"default", "full"}:
        return StampValidation(False, f"invalid required tier: {required_tier!r}")
    if required_tier == "full" and stamp.tier != "full":
        return StampValidation(False, "default stamp does not satisfy full release")
    if not _clean_tree(repo):
        return StampValidation(False, "public working tree is dirty")
    try:
        record = verify_local_install(repo)
    except EnvironmentError as error:
        return StampValidation(False, str(error))
    if stamp.local_tooling_commit != record.commit:
        return StampValidation(False, "stamp local tooling commit does not match install")
    try:
        toolchain = current_toolchain(repo, env)
    except EnvironmentError as error:
        return StampValidation(False, str(error))
    if stamp.toolchain != toolchain:
        return StampValidation(False, "stamp toolchain does not match current rustc")
    if pushed_tree is None:
        pushed_tree = _git_output(repo, ["rev-parse", "HEAD^{tree}"])
    if stamp.tree != pushed_tree:
        return StampValidation(False, "stamp tree does not match pushed tree")
    try:
        commit_tree = _git_output(repo, ["rev-parse", f"{stamp.commit}^{{tree}}"])
    except subprocess.CalledProcessError:
        return StampValidation(False, "stamp commit is unavailable")
    if commit_tree != stamp.tree:
        return StampValidation(False, "stamp commit does not resolve to stamp tree")
    try:
        expected_hash = compute_steps_hash(repo, stamp.tier)
    except EnvironmentError as error:
        return StampValidation(False, str(error))
    if stamp.steps_hash != expected_hash:
        return StampValidation(False, "stamp steps hash does not match current tooling")
    if release:
        if stamp.tier != "full":
            return StampValidation(False, "release requires a full stamp")
        if pushed_commit is None or stamp.commit != pushed_commit:
            return StampValidation(False, "release commit does not match full stamp commit")
    return StampValidation(True, "gate stamp accepted")


def _resolve_tip(repo: Path, object_id: str) -> tuple[str, str]:
    commit = _git_output(repo, ["rev-parse", f"{object_id}^{{commit}}"])
    tree = _git_output(repo, ["rev-parse", f"{object_id}^{{tree}}"])
    return commit, tree


def authorize_push(
    repo: Path,
    remote_name: str,
    remote_url: str,
    updates_text: str,
    *,
    env: Mapping[str, str] | None = None,
    auto_gate: bool = False,
) -> PushAuthorization:
    """Authorize every ref, automatically gating only the current HEAD tip."""

    del remote_name
    env_map = os.environ if env is None else env
    updates: list[tuple[str, str, str, str]] = []
    for line_number, line in enumerate(updates_text.splitlines(), start=1):
        if not line.strip():
            continue
        fields = line.split()
        if len(fields) != 4:
            return PushAuthorization(
                False, f"invalid pre-push update on line {line_number}"
            )
        updates.append((fields[0], fields[1], fields[2], fields[3]))

    non_deletions = [
        (local_ref, local_sha, remote_ref, remote_sha)
        for local_ref, local_sha, remote_ref, remote_sha in updates
        if not _ZERO_OBJECT.fullmatch(local_sha)
    ]
    github_remote = "github.com" in remote_url.lower()
    if github_remote and any(
        local_ref.startswith("refs/heads/local/")
        or remote_ref.startswith("refs/heads/local/")
        for local_ref, _local_sha, remote_ref, _remote_sha in updates
    ):
        return PushAuthorization(
            False, "pre-push: refs/heads/local/* must never be pushed to GitHub"
        )

    if not non_deletions:
        return PushAuthorization(True, "pre-push: deletions require no gate stamp")

    resolved: list[tuple[str, str, str, str, str]] = []
    for local_ref, local_sha, remote_ref, remote_sha in non_deletions:
        try:
            commit, tree = _resolve_tip(repo, local_sha)
        except subprocess.CalledProcessError:
            return PushAuthorization(
                False, f"pre-push: {remote_ref} does not resolve to a commit tree"
            )
        resolved.append((local_ref, remote_ref, remote_sha, commit, tree))

    if not _clean_tree(repo):
        return PushAuthorization(False, "pre-push: public working tree is dirty")
    head_commit = _git_output(repo, ["rev-parse", "HEAD"])
    current = [entry for entry in resolved if entry[3] == head_commit]
    non_head = [entry for entry in resolved if entry[3] != head_commit]
    stamp = load_stamp(repo)

    for _local_ref, remote_ref, _remote_sha, commit, tree in non_head:
        release = _RELEASE_REF.match(remote_ref) is not None
        validation = validate_stamp(
            repo,
            stamp,
            required_tier="full" if release else "default",
            pushed_tree=tree,
            pushed_commit=commit,
            release=release,
            env=env_map,
        )
        if not validation.valid:
            return PushAuthorization(
                False,
                (
                    f"pre-push: non-HEAD tip {remote_ref} requires an accepted "
                    f"stamp; check it out and gate it: {validation.message}"
                ),
            )

    invalid_current: list[tuple[str, StampValidation]] = []
    for _local_ref, remote_ref, _remote_sha, commit, tree in current:
        release = _RELEASE_REF.match(remote_ref) is not None
        validation = validate_stamp(
            repo,
            stamp,
            required_tier="full" if release else "default",
            pushed_tree=tree,
            pushed_commit=commit,
            release=release,
            env=env_map,
        )
        if not validation.valid:
            invalid_current.append((remote_ref, validation))

    if invalid_current:
        release_failure = next(
            (
                (remote_ref, validation)
                for remote_ref, validation in invalid_current
                if _RELEASE_REF.match(remote_ref) is not None
            ),
            None,
        )
        if release_failure is not None:
            remote_ref, validation = release_failure
            return PushAuthorization(
                False, f"pre-push: {remote_ref}: {validation.message}"
            )
        if not auto_gate or not current:
            remote_ref, validation = invalid_current[0]
            return PushAuthorization(
                False, f"pre-push: {remote_ref}: {validation.message}"
            )
        head_tree = current[0][4]
        if any(tree != head_tree for _a, _b, _c, _d, tree in non_head):
            remote_ref, _validation = invalid_current[0]
            return PushAuthorization(
                False,
                (
                    f"pre-push: {remote_ref} cannot be gated automatically while "
                    "the same push contains a different non-HEAD tree"
                ),
            )

        delta_trees: list[str] = []
        for _local_ref, _remote_ref, remote_sha, _commit, _tree in current:
            if _ZERO_OBJECT.fullmatch(remote_sha):
                continue
            try:
                remote_tree = _git_output(
                    repo, ["rev-parse", f"{remote_sha}^{{tree}}"]
                )
            except subprocess.CalledProcessError:
                continue
            if remote_tree not in delta_trees:
                delta_trees.append(remote_tree)
        try:
            registry = load_registry(repo)
        except EnvironmentError as error:
            return PushAuthorization(False, f"pre-push: environment: {error}")
        result = execute_gate(
            repo,
            registry,
            tier="default",
            named_step=None,
            env=env_map,
            delta_base_tree=delta_trees[0] if delta_trees else None,
            additional_delta_trees=tuple(delta_trees[1:]),
        )
        if result != 0:
            return PushAuthorization(
                False, "pre-push: automatic gate for current HEAD failed"
            )
        stamp = load_stamp(repo)

    for _local_ref, remote_ref, _remote_sha, commit, tree in resolved:
        release = _RELEASE_REF.match(remote_ref) is not None
        validation = validate_stamp(
            repo,
            stamp,
            required_tier="full" if release else "default",
            pushed_tree=tree,
            pushed_commit=commit,
            release=release,
            env=env_map,
        )
        if not validation.valid:
            return PushAuthorization(
                False, f"pre-push: {remote_ref}: {validation.message}"
            )
    return PushAuthorization(
        True, f"pre-push: gate stamp authorizes {len(resolved)} pushed tip(s)"
    )


@dataclass(frozen=True)
class GateRegistry:
    """Validated single source of gate steps and retention policy."""

    steps: tuple[Step, ...]
    documentation_allowlist: tuple[str, ...]
    registered_files: tuple[dict[str, object], ...]
    max_runs: int
    max_age_days: int


def load_registry(repo: Path) -> GateRegistry:
    """Load and validate the installed step registry."""

    path = _validated_repo_file(
        repo, "scripts/gate-steps.json", label="gate step registry"
    )
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EnvironmentError(f"invalid gate step registry: {error}") from error
    if not isinstance(payload, dict) or payload.get("schema") != 1:
        raise EnvironmentError("invalid gate step registry schema")
    raw_steps = payload.get("steps")
    retention = payload.get("retention")
    allowlist = payload.get("documentation_allowlist")
    registered_files = payload.get("registered_files")
    if (
        not isinstance(raw_steps, list)
        or not raw_steps
        or not isinstance(retention, dict)
        or type(retention.get("max_runs")) is not int
        or not 1 <= retention["max_runs"] <= 1000
        or type(retention.get("max_age_days")) is not int
        or not 1 <= retention["max_age_days"] <= 3650
        or not isinstance(allowlist, list)
        or not allowlist
        or any(not isinstance(item, str) or not item for item in allowlist)
        or len(set(allowlist)) != len(allowlist)
        or not isinstance(registered_files, list)
    ):
        raise EnvironmentError("invalid gate step registry definitions")

    registered_paths: set[str] = set()
    normalized_registered: list[dict[str, object]] = []
    for entry in registered_files:
        if (
            not isinstance(entry, dict)
            or set(entry) != {"path", "checks"}
            or not isinstance(entry["path"], str)
            or len(Path(entry["path"]).parts) != 1
            or "\\" in entry["path"]
            or entry["path"] in {".", ".."}
            or entry["path"].startswith(("~", "/"))
            or any(character in entry["path"] for character in "*?[")
            or not isinstance(entry["checks"], list)
            or not entry["checks"]
            or any(not isinstance(check, str) for check in entry["checks"])
            or not {"utf8", "doc-references", "prose-dashes"}.issubset(
                set(entry["checks"])
            )
        ):
            raise EnvironmentError("invalid registered-file gate definition")
        if entry["path"] in registered_paths:
            raise EnvironmentError(f"duplicate registered file: {entry['path']}")
        registered_paths.add(entry["path"])
        normalized_registered.append(
            {"path": entry["path"], "checks": list(entry["checks"])}
        )
    built_in_allowlist = {"manual/**", ".manual-translations.lock"}
    if not built_in_allowlist.issubset(allowlist):
        raise EnvironmentError("documentation allowlist omits required manual paths")
    allowlisted_roots = set(allowlist) - built_in_allowlist
    missing_registrations = allowlisted_roots - registered_paths
    unused_registrations = registered_paths - allowlisted_roots
    if missing_registrations:
        raise EnvironmentError(
            "allowlisted root path lacks registered checks: "
            + ", ".join(sorted(missing_registrations))
        )
    if unused_registrations:
        raise EnvironmentError(
            "registered root path is absent from documentation allowlist: "
            + ", ".join(sorted(unused_registrations))
        )

    steps: list[Step] = []
    ids: set[str] = set()
    for raw in raw_steps:
        if not isinstance(raw, dict):
            raise EnvironmentError("invalid gate step entry")
        required = {
            "id",
            "name",
            "tiers",
            "argv",
            "timeout_seconds",
            "category",
            "capabilities",
        }
        if set(raw) != required:
            raise EnvironmentError("invalid gate step fields")
        step_id = raw["id"]
        tiers = raw["tiers"]
        argv = raw["argv"]
        capabilities = raw["capabilities"]
        if (
            not isinstance(step_id, str)
            or not _SAFE_STEP_ID.fullmatch(step_id)
            or step_id in ids
            or not isinstance(raw["name"], str)
            or not raw["name"]
            or not isinstance(tiers, list)
            or not tiers
            or any(tier not in {"default", "full", "docs"} for tier in tiers)
            or len(set(tiers)) != len(tiers)
            or not isinstance(argv, list)
            or not argv
            or any(not isinstance(argument, str) or not argument for argument in argv)
            or type(raw["timeout_seconds"]) is not int
            or raw["timeout_seconds"] <= 0
            or raw["category"] not in {"docs", "code", "environment"}
            or not isinstance(capabilities, list)
            or any(
                not isinstance(capability, str) or not capability
                for capability in capabilities
            )
        ):
            raise EnvironmentError(f"invalid gate step definition: {step_id!r}")
        _validate_step_executable_path(argv[0])
        if "default" in tiers and "full" not in tiers:
            raise EnvironmentError(f"default step must be inherited by full: {step_id}")
        ids.add(step_id)
        steps.append(
            Step(
                id=step_id,
                name=raw["name"],
                tiers=tuple(tiers),
                argv=tuple(argv),
                timeout_seconds=raw["timeout_seconds"],
                category=raw["category"],
                capabilities=tuple(capabilities),
            )
        )
    return GateRegistry(
        steps=tuple(steps),
        documentation_allowlist=tuple(allowlist),
        registered_files=tuple(normalized_registered),
        max_runs=retention["max_runs"],
        max_age_days=retention["max_age_days"],
    )


def _documentation_paths(registry: GateRegistry) -> frozenset[str]:
    registered = {
        str(entry["path"])
        for entry in registry.registered_files
        if isinstance(entry.get("path"), str)
    }
    allowlisted = set(registry.documentation_allowlist)
    return frozenset(
        {".manual-translations.lock"} | (registered & allowlisted)
    )


def _classify_delta(
    repo: Path,
    registry: GateRegistry,
    code_tree: str,
    head_tree: str,
) -> DeltaClass:
    output = _git_output(
        repo,
        ["diff", "--name-only", "--no-renames", code_tree, head_tree, "--"],
    )
    paths = tuple(path for path in output.splitlines() if path)
    if not paths:
        return DeltaClass.EMPTY
    root_docs = _documentation_paths(registry)
    docs = tuple(
        path.startswith("manual/") or path in root_docs
        for path in paths
    )
    if all(docs):
        return DeltaClass.DOCS_ONLY
    if any(docs):
        return DeltaClass.MIXED
    return DeltaClass.CODE


def classify_delta(code_tree: str, head_tree: str) -> DeltaClass:
    """Classify a complete net delta using the current repository registry."""

    repo = Path.cwd()
    return _classify_delta(repo, load_registry(repo), code_tree, head_tree)


def _invalid_inheritance(
    message: str,
    delta_class: DeltaClass = DeltaClass.CODE,
) -> InheritanceDecision:
    return InheritanceDecision(False, delta_class, None, message)


def validate_documentation_inheritance(
    repo: Path,
    stamp: GateStamp | None,
    *,
    env: Mapping[str, str] | None = None,
) -> InheritanceDecision:
    """Validate a non-transitive docs-only inheritance decision."""

    if stamp is None or _stamp_from_payload(asdict(stamp)) is None:
        return _invalid_inheritance("missing or invalid default/full base stamp")
    if not _clean_tree(repo):
        return _invalid_inheritance("public working tree is dirty")
    try:
        install = verify_local_install(repo)
    except EnvironmentError as error:
        return _invalid_inheritance(str(error))
    if stamp.local_tooling_commit != install.commit:
        return _invalid_inheritance(
            "base stamp local tooling commit does not match install"
        )
    try:
        toolchain = current_toolchain(repo, env)
    except EnvironmentError as error:
        return _invalid_inheritance(str(error))
    if stamp.toolchain != toolchain:
        return _invalid_inheritance(
            "base stamp toolchain does not match current rustc"
        )
    try:
        steps_hash = compute_steps_hash(repo, stamp.tier)
    except EnvironmentError as error:
        return _invalid_inheritance(str(error))
    if stamp.steps_hash != steps_hash:
        return _invalid_inheritance(
            "base stamp steps hash does not match current tooling"
        )
    try:
        commit_tree = _git_output(repo, ["rev-parse", f"{stamp.commit}^{{tree}}"])
    except subprocess.CalledProcessError:
        return _invalid_inheritance("base stamp commit is unavailable")
    if commit_tree != stamp.tree:
        return _invalid_inheritance(
            "base stamp commit does not resolve to base stamp tree"
        )
    ancestor = subprocess.run(
        ["git", "merge-base", "--is-ancestor", stamp.commit, "HEAD"],
        cwd=repo,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if ancestor.returncode != 0:
        return _invalid_inheritance("base stamp commit is not an ancestor of HEAD")

    provenance = resolve_code_provenance(stamp)
    try:
        object_type = _git_output(repo, ["cat-file", "-t", provenance])
    except subprocess.CalledProcessError:
        return _invalid_inheritance(
            f"original code provenance tree is unavailable: {provenance}"
        )
    if object_type != "tree":
        return _invalid_inheritance(
            f"original code provenance is not a tree object: {provenance}"
        )
    try:
        head_tree = _git_output(repo, ["rev-parse", "HEAD^{tree}"])
        delta_class = _classify_delta(
            repo, load_registry(repo), provenance, head_tree
        )
    except (EnvironmentError, subprocess.CalledProcessError) as error:
        return _invalid_inheritance(f"cannot classify documentation delta: {error}")
    if delta_class is DeltaClass.EMPTY:
        return _invalid_inheritance(
            "documentation inheritance requires a nonempty net delta",
            delta_class,
        )
    if delta_class is not DeltaClass.DOCS_ONLY:
        return _invalid_inheritance(
            f"net delta from original code tree is {delta_class.value}, not docs-only",
            delta_class,
        )
    return InheritanceDecision(
        True,
        delta_class,
        provenance,
        (
            f"code verification inherited from tree {provenance}; "
            "code verification was not executed on the current tree"
        ),
    )


def _merge_delta_classes(classes: tuple[DeltaClass, ...]) -> DeltaClass:
    has_docs = any(
        value in {DeltaClass.DOCS_ONLY, DeltaClass.MIXED} for value in classes
    )
    has_code = any(value in {DeltaClass.CODE, DeltaClass.MIXED} for value in classes)
    if has_docs and has_code:
        return DeltaClass.MIXED
    if has_docs:
        return DeltaClass.DOCS_ONLY
    if has_code:
        return DeltaClass.CODE
    return DeltaClass.EMPTY


def _parent_or_empty_tree(repo: Path) -> str | None:
    try:
        return _git_output(repo, ["rev-parse", "HEAD^1^{tree}"])
    except subprocess.CalledProcessError:
        try:
            return subprocess.check_output(
                ["git", "mktree"],
                cwd=repo,
                input="",
                text=True,
                stderr=subprocess.DEVNULL,
            ).strip()
        except subprocess.CalledProcessError:
            return None


def select_gate_plan(
    repo: Path,
    registry: GateRegistry,
    *,
    tier: str,
    env: Mapping[str, str],
    base_stamp: GateStamp | None = None,
    delta_base_tree: str | None = None,
    additional_delta_trees: tuple[str, ...] = (),
) -> GatePlan:
    """Select the exact ordered steps for full, code, mixed, or docs-only work."""

    if tier == "full":
        return GatePlan(
            steps=tuple(step for step in registry.steps if "full" in step.tiers),
            delta_class=DeltaClass.CODE,
            inherited=False,
            code_provenance=None,
            message="full gate executes every full-tier step",
        )
    if tier != "default":
        raise EnvironmentError(f"invalid gate tier: {tier!r}")

    inheritance = validate_documentation_inheritance(
        repo, base_stamp, env=env
    )
    if inheritance.valid:
        return GatePlan(
            steps=tuple(step for step in registry.steps if "docs" in step.tiers),
            delta_class=inheritance.delta_class,
            inherited=True,
            code_provenance=inheritance.code_provenance,
            message=inheritance.message,
        )

    head_tree = _git_output(repo, ["rev-parse", "HEAD^{tree}"])
    candidates: list[str] = []
    for candidate in (delta_base_tree, *additional_delta_trees):
        if candidate and candidate not in candidates:
            candidates.append(candidate)
    if base_stamp is not None:
        provenance = resolve_code_provenance(base_stamp)
        try:
            if (
                _git_output(repo, ["cat-file", "-t", provenance]) == "tree"
                and provenance not in candidates
            ):
                candidates.append(provenance)
        except subprocess.CalledProcessError:
            pass
    if not candidates:
        parent = _parent_or_empty_tree(repo)
        if parent is not None:
            candidates.append(parent)

    classifications: list[DeltaClass] = []
    for candidate in candidates:
        try:
            classifications.append(
                _classify_delta(repo, registry, candidate, head_tree)
            )
        except subprocess.CalledProcessError:
            classifications.append(DeltaClass.CODE)
    delta_class = _merge_delta_classes(tuple(classifications))
    selected_tiers = {"default"}
    if delta_class in {DeltaClass.DOCS_ONLY, DeltaClass.MIXED}:
        selected_tiers.add("docs")
    return GatePlan(
        steps=tuple(
            step
            for step in registry.steps
            if selected_tiers.intersection(step.tiers)
        ),
        delta_class=delta_class,
        inherited=False,
        code_provenance=None,
        message=(
            f"{delta_class.value} delta: code verification will execute; "
            f"inheritance unavailable ({inheritance.message})"
        ),
    )


def _process_start_ticks(pid: int) -> int | None:
    try:
        raw = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    except (FileNotFoundError, PermissionError, ProcessLookupError):
        return None
    marker = raw.rfind(") ")
    if marker < 0:
        return None
    fields = raw[marker + 2 :].split()
    if len(fields) <= 19:
        return None
    return int(fields[19])


def _active_path(repo: Path) -> Path:
    return _git_path(repo, "suprnova-gate-active.json")


def _remove_active_if_owned(repo: Path, run_id: str) -> None:
    path = _active_path(repo)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return
    if isinstance(payload, dict) and payload.get("run_id") == run_id:
        path.unlink(missing_ok=True)


def _recover_or_reject_active(
    repo: Path, env: Mapping[str, str], grace_seconds: float
) -> str | None:
    path = _active_path(repo)
    if not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        run_id = payload["run_id"]
        pid = int(payload["pid"])
        start_ticks = int(payload["process_start_ticks"])
        raw_container_cli = payload.get("container_cli")
        if (
            not isinstance(run_id, str)
            or not run_id
            or (
                raw_container_cli is not None
                and (
                    not isinstance(raw_container_cli, list)
                    or any(not isinstance(item, str) for item in raw_container_cli)
                )
            )
        ):
            raise ValueError
    except (OSError, json.JSONDecodeError, KeyError, TypeError, ValueError):
        path.unlink(missing_ok=True)
        return "stale run metadata was invalid and has been removed"
    if _process_start_ticks(pid) == start_ticks:
        return f"gate run {run_id} is already active in runner PID {pid}"

    process_leaks = _scan_run_processes(run_id)
    if process_leaks:
        _terminate_leaked_processes(process_leaks, run_id, grace_seconds)
    container_cli = (
        tuple(raw_container_cli) if raw_container_cli is not None else None
    )
    stale_context = RunContext(
        repo=repo,
        run_id=run_id,
        run_dir=_git_path(repo, f"suprnova-gate-runs/{run_id}"),
        tier="default",
        env=env,
        termination_grace_seconds=grace_seconds,
        container_cli=container_cli,
    )
    container_leaks, cleanup_error = _cleanup_containers(stale_context)
    path.unlink(missing_ok=True)
    identities: list[str] = [
        f"process {leak['pid']} ({leak.get('command', '')})" for leak in process_leaks
    ]
    identities.extend(
        f"container {leak['id']} ({leak['image']})" for leak in container_leaks
    )
    detail = ", ".join(identities) if identities else "no surviving resource identity"
    if cleanup_error:
        detail += f"; {cleanup_error}"
    return f"stale run {run_id} detected and cleaned: {detail}"


def _acquire_active(
    repo: Path, run_id: str, container_cli: tuple[str, ...] | None
) -> None:
    payload = {
        "schema": 1,
        "run_id": run_id,
        "pid": os.getpid(),
        "process_start_ticks": _process_start_ticks(os.getpid()),
        "started_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "container_cli": list(container_cli) if container_cli is not None else None,
    }
    path = _active_path(repo)
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())


def _preflight(
    repo: Path,
    steps: tuple[Step, ...],
    env: Mapping[str, str],
) -> str | None:
    try:
        install = verify_local_install(repo)
    except EnvironmentError as error:
        return str(error)
    missing = sorted(
        {
            capability
            for step in steps
            for capability in step.capabilities
            if shutil.which(capability, path=env.get("PATH")) is None
        }
    )
    if shutil.which("rustc", path=env.get("PATH")) is None:
        missing.append("rustc")
    if missing:
        return "missing required capability: " + ", ".join(sorted(set(missing)))
    repo_root = repo.resolve()
    for step in steps:
        for index, argument in enumerate(step.argv):
            executable = index == 0
            is_path_executable = executable and (
                "/" in argument
                or "\\" in argument
                or argument.startswith("~")
            )
            asset = (
                argument
                if is_path_executable
                else _repository_local_gate_argument(repo_root, argument)
            )
            if asset is not None:
                error = _preflight_installed_asset(
                    repo,
                    repo_root,
                    install,
                    asset,
                    executable=executable,
                )
                if error is not None:
                    return error
            elif executable and shutil.which(argument, path=env.get("PATH")) is None:
                return f"missing step executable: {argument}"
    if any("docker" in step.capabilities for step in steps):
        try:
            docker = subprocess.run(
                ["docker", "info"],
                cwd=repo,
                env=dict(env),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=15,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            return f"Docker capability unavailable: {error}"
        if docker.returncode != 0:
            return "Docker capability unavailable: docker info failed"
    return None


def _new_run_id() -> str:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    return f"{timestamp}-{os.getpid()}-{secrets.token_hex(4)}"


def _write_summary(run_dir: Path, summary: dict[str, object]) -> None:
    path = run_dir / "summary.json"
    temporary = path.with_name(f"{path.name}.tmp-{os.getpid()}")
    temporary.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.replace(temporary, path)


def _read_summary(path: Path) -> dict[str, object] | None:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return payload if isinstance(payload, dict) else None


def _find_outcome_comparison(
    runs_root: Path, current: dict[str, object]
) -> str | None:
    current_results = current.get("results")
    tree = current.get("tree")
    steps_hash = current.get("steps_hash")
    run_id = current.get("run_id")
    if (
        not isinstance(current_results, list)
        or not tree
        or not steps_hash
        or not isinstance(run_id, str)
    ):
        return None
    by_step = {
        result.get("step"): result.get("outcome")
        for result in current_results
        if isinstance(result, dict)
    }
    candidates = sorted(
        (
            path
            for path in runs_root.iterdir()
            if path.is_dir() and path.name != run_id
        ),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    for directory in candidates:
        previous = _read_summary(directory / "summary.json")
        if (
            previous is None
            or previous.get("tree") != tree
            or previous.get("steps_hash") != steps_hash
            or not isinstance(previous.get("results"), list)
        ):
            continue
        for result in previous["results"]:
            if not isinstance(result, dict):
                continue
            step = result.get("step")
            prior_outcome = result.get("outcome")
            current_outcome = by_step.get(step)
            if current_outcome is not None and current_outcome != prior_outcome:
                return (
                    f"environmental fault: step {step} changed outcome "
                    f"{prior_outcome} in run {directory.name} -> "
                    f"{current_outcome} in run {run_id}"
                )
    return None


def prune_run_history(
    runs_root: Path, *, max_runs: int, max_age_days: int, current_run_id: str
) -> None:
    """Bound history while preferring the newest run for each tree/hash pair."""

    if not runs_root.exists():
        return
    directories = [path for path in runs_root.iterdir() if path.is_dir()]
    now = time.time()
    age_limit = max_age_days * 86400
    for directory in list(directories):
        if directory.name == current_run_id:
            continue
        if now - directory.stat().st_mtime > age_limit:
            shutil.rmtree(directory, ignore_errors=True)
            directories.remove(directory)
    directories.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    newest_pairs: set[tuple[object, object]] = set()
    protected: set[Path] = set()
    for directory in directories:
        summary = _read_summary(directory / "summary.json")
        pair = (
            summary.get("tree") if summary else None,
            summary.get("steps_hash") if summary else None,
        )
        if pair != (None, None) and pair not in newest_pairs:
            newest_pairs.add(pair)
            protected.add(directory)
    removal_order = [
        directory
        for directory in reversed(directories)
        if directory not in protected and directory.name != current_run_id
    ]
    removal_order.extend(
        directory
        for directory in reversed(directories)
        if directory in protected and directory.name != current_run_id
    )
    while len(directories) > max_runs and removal_order:
        victim = removal_order.pop(0)
        if victim not in directories:
            continue
        shutil.rmtree(victim, ignore_errors=True)
        directories.remove(victim)


def _redact_terminal_text(text: str) -> str:
    redacted = _URL_USERINFO_SECRET.sub(r"\1[REDACTED]", text)
    return _PASSWORD_LIKE_SECRET.sub(r"\1[REDACTED]", redacted)


def _tail_log(path: str, max_lines: int = 40, max_bytes: int = 8192) -> str:
    try:
        data = Path(path).read_bytes()
    except OSError:
        return ""
    text = data[-max_bytes:].decode(errors="replace")
    tail = "\n".join(text.splitlines()[-max_lines:])
    return _redact_terminal_text(tail)


def _print_step_header() -> None:
    print(f"{'STEP':<28} {'OUTCOME':<13} {'SECONDS':>8}   LOG")


def _print_step_result(result: StepResult) -> None:
    print(
        f"{result.step:<28} {result.outcome.value:<13} "
        f"{result.seconds:>8.2f}   {result.log_path}"
    )
    if result.outcome is Outcome.PASS:
        return
    print(f"  command: {' '.join(result.argv)}")
    print(f"  log: {result.log_path}")
    tail = _tail_log(result.log_path)
    if tail:
        print("  log tail:")
        for line in tail.splitlines():
            print(f"    {line}")
    print(f"  diagnose: python3 scripts/gate-runner.py --step {result.step}")
    if result.leaks:
        print("  environment poisoned - do not run again until resources are gone")
        for leak in result.leaks:
            detail = _redact_terminal_text(json.dumps(leak, sort_keys=True))
            print(f"    leaked {detail}")


_EXIT_CODES = {
    Outcome.PASS: 0,
    Outcome.FAIL: 1,
    Outcome.TIMEOUT: 124,
    Outcome.ENVIRONMENT: 2,
    Outcome.INTERRUPTED: 130,
    Outcome.LEAK_DETECTED: 3,
}


class _GateEnvironment(Exception):
    """Abort a run before or between steps with an environment verdict."""


class _GateInterrupted(Exception):
    """Transfer signal state from a safe checkpoint to terminal finalization."""



def _safe_unlink_stamp(repo: Path) -> None:
    _git_path(repo, "suprnova-gate-pass").unlink(missing_ok=True)


def execute_gate(
    repo: Path,
    registry: GateRegistry,
    *,
    tier: str,
    named_step: str | None,
    env: Mapping[str, str],
    delta_base_tree: str | None = None,
    additional_delta_trees: tuple[str, ...] = (),
) -> int:
    """Execute one authoritative tier or a non-stamping diagnostic step."""

    plan: GatePlan | None = None
    if named_step is None:
        if not _clean_tree(repo):
            print("environment: public working tree is dirty; no stamp consumed")
            print("GATE FAILED: environment")
            return _EXIT_CODES[Outcome.ENVIRONMENT]
        plan = select_gate_plan(
            repo,
            registry,
            tier=tier,
            env=env,
            base_stamp=load_stamp(repo),
            delta_base_tree=delta_base_tree,
            additional_delta_trees=additional_delta_trees,
        )
        selected = plan.steps
    else:
        matches = tuple(step for step in registry.steps if step.id == named_step)
        if not matches:
            print(f"environment: unknown gate step: {named_step}", file=sys.stderr)
            print("GATE FAILED: environment")
            return _EXIT_CODES[Outcome.ENVIRONMENT]
        selected = matches
    run_id = _new_run_id()
    runs_root = _git_path(repo, "suprnova-gate-runs")
    run_dir = runs_root / run_id
    run_dir.mkdir(parents=True, exist_ok=False, mode=0o700)
    run_dir.chmod(0o700)

    def finish() -> None:
        prune_run_history(
            runs_root,
            max_runs=registry.max_runs,
            max_age_days=registry.max_age_days,
            current_run_id=run_id,
        )

    interrupted = threading.Event()

    def handle_signal(_signum: int, _frame: object) -> None:
        interrupted.set()

    previous_handlers = {
        signal_number: signal.signal(signal_number, handle_signal)
        for signal_number in (signal.SIGINT, signal.SIGTERM)
    }
    started_at = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    tree: str | None = None
    steps_hash: str | None = None
    results: list[StepResult] = []
    terminal_message: str | None = None
    overall = Outcome.PASS
    active_owned = False
    container_cli: tuple[str, ...] | None = None
    scope_message = plan.message if plan is not None else None

    def raise_if_interrupted(phase: str) -> None:
        if interrupted.is_set():
            raise _GateInterrupted(f"gate interrupted during {phase}")

    def apply_pending_interrupt(phase: str) -> bool:
        nonlocal overall, terminal_message
        if not interrupted.is_set() or overall is Outcome.INTERRUPTED:
            return False
        overall = Outcome.INTERRUPTED
        terminal_message = f"gate interrupted during {phase}"
        if named_step is None:
            _safe_unlink_stamp(repo)
        return True

    def current_summary() -> dict[str, object]:
        return {
            "schema": 1,
            "run_id": run_id,
            "tier": tier,
            "tree": tree,
            "steps_hash": steps_hash,
            "started_at": started_at,
            "finished_at": datetime.now(timezone.utc)
            .isoformat()
            .replace("+00:00", "Z"),
            "outcome": overall.value,
            "message": terminal_message,
            "delta_class": plan.delta_class.value if plan is not None else None,
            "inherited": plan.inherited if plan is not None else False,
            "code_provenance": (
                plan.code_provenance if plan is not None else None
            ),
            "results": [result.to_json() for result in results],
        }

    try:
        try:
            if named_step is None:
                _safe_unlink_stamp(repo)
            raise_if_interrupted("initialization")

            stale_message = _recover_or_reject_active(repo, env, 2.0)
            raise_if_interrupted("stale-run recovery")
            if stale_message is not None:
                raise _GateEnvironment(stale_message)

            preflight_error = _preflight(repo, selected, env)
            raise_if_interrupted("preflight")
            if preflight_error is not None:
                raise _GateEnvironment(preflight_error)

            try:
                tree = _git_output(repo, ["rev-parse", "HEAD^{tree}"])
                raise_if_interrupted("tree resolution")
                steps_hash = compute_steps_hash(repo, tier)
                raise_if_interrupted("gate-definition hashing")
            except (EnvironmentError, subprocess.CalledProcessError) as error:
                raise _GateEnvironment(str(error)) from error

            uses_docker = any("docker" in step.capabilities for step in selected)
            container_cli = ("docker",) if uses_docker else None
            try:
                _acquire_active(repo, run_id, container_cli)
                active_owned = True
            except FileExistsError as error:
                raise _GateEnvironment(
                    "another gate runner acquired the active-run record"
                ) from error
            raise_if_interrupted("active-run acquisition")

            context = RunContext(
                repo=repo,
                run_id=run_id,
                run_dir=run_dir,
                tier=tier,
                env=env,
                termination_grace_seconds=2.0,
                interrupt_event=interrupted,
                container_cli=container_cli,
            )
            for step in selected:
                raise_if_interrupted("step dispatch")
                result = run_step(step, context)
                results.append(result)
                if result.outcome is not Outcome.PASS:
                    overall = result.outcome
                    terminal_message = result.message
                    break
                raise_if_interrupted("step execution")

            if overall is Outcome.PASS and named_step is None:
                try:
                    stamp = build_stamp(
                        repo,
                        tier=tier,
                        run_id=run_id,
                        code_provenance=(
                            plan.code_provenance if plan is not None else None
                        ),
                        env=env,
                    )
                    raise_if_interrupted("stamp construction")
                    if stamp.tree != tree or stamp.steps_hash != steps_hash:
                        raise EnvironmentError(
                            "tree or gate definitions changed while gate was running"
                        )
                    write_stamp(repo, stamp)
                    raise_if_interrupted("stamp write")
                except EnvironmentError as error:
                    raise _GateEnvironment(str(error)) from error
        except _GateInterrupted as error:
            overall = Outcome.INTERRUPTED
            terminal_message = str(error)
            if named_step is None:
                _safe_unlink_stamp(repo)
        except _GateEnvironment as error:
            overall = Outcome.ENVIRONMENT
            terminal_message = str(error)
            if named_step is None:
                _safe_unlink_stamp(repo)
        except Exception as error:
            overall = Outcome.ENVIRONMENT
            terminal_message = (
                f"runner internal error: {type(error).__name__}: {error}"
            )
            if named_step is None:
                _safe_unlink_stamp(repo)
        finally:
            if active_owned:
                _remove_active_if_owned(repo, run_id)
            apply_pending_interrupt("resource cleanup")

        summary = current_summary()
        _write_summary(run_dir, summary)
        finish()
        comparison = _find_outcome_comparison(runs_root, summary)
        if apply_pending_interrupt("finalization"):
            summary = current_summary()
            _write_summary(run_dir, summary)
            finish()
            comparison = _find_outcome_comparison(runs_root, summary)
        if results:
            _print_step_header()
            for result in results:
                _print_step_result(result)
        if terminal_message and not results:
            print(f"{overall.value}: {terminal_message}")
        if comparison:
            print(comparison)
        if overall is Outcome.PASS and scope_message:
            print(f"scope: {scope_message}")
        if overall is Outcome.PASS:
            print(f"GATE GREEN: {tier}, tree {tree}, run {run_id}")
        else:
            print(f"GATE FAILED: {overall.value}")
        return _EXIT_CODES[overall]
    finally:
        for signal_number, previous in previous_handlers.items():
            signal.signal(signal_number, previous)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--full", action="store_true", help="run the full tier")
    selection.add_argument("--step", metavar="ID", help="run one diagnostic step")
    parser.add_argument(
        "--authorize-push",
        nargs=2,
        metavar=("REMOTE_NAME", "REMOTE_URL"),
        help="authorize pre-push ref updates read from stdin",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        repo = Path(_git_output(Path.cwd(), ["rev-parse", "--show-toplevel"]))
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"environment: not inside a Git worktree: {error}", file=sys.stderr)
        print("GATE FAILED: environment")
        return _EXIT_CODES[Outcome.ENVIRONMENT]
    if args.authorize_push is not None:
        remote_name, remote_url = args.authorize_push
        decision = authorize_push(
            repo,
            remote_name,
            remote_url,
            sys.stdin.read(),
            env=os.environ,
            auto_gate=True,
        )
        print(decision.message)
        return 0 if decision.allowed else 1
    try:
        registry = load_registry(repo)
    except EnvironmentError as error:
        print(f"environment: {error}", file=sys.stderr)
        print("GATE FAILED: environment")
        return _EXIT_CODES[Outcome.ENVIRONMENT]
    return execute_gate(
        repo,
        registry,
        tier="full" if args.full else "default",
        named_step=args.step,
        env=os.environ,
    )


if __name__ == "__main__":
    raise SystemExit(main())
