"""Run one shared Live contract observer without turning missing evidence into a pass."""

import argparse
import json
import os
from pathlib import Path
import secrets
import selectors
import signal
import subprocess
import sys
import time

from live_contract_checks.acceptance import Incomplete, OBSERVERS, Violation, evaluate, fields, need
from live_contract_checks.identity import checkout_identity, sdk_checkout


OUTPUT_LIMIT = 1024 * 1024
TIMEOUT_SECONDS = 600
FRAMEWORK_INPUTS = ("Cargo.toml", "Cargo.lock", ".cargo/config.toml", "crates", "framework",
                    "suprnova-cli", "suprnova-macros", "live_contract_checks", "proof")


def unique_object(pairs: list) -> dict:
    result = {}
    for key, value in pairs:
        need(key not in result, "duplicate JSON key in observer input")
        result[key] = value
    return result


def parse_json(content: bytes) -> dict:
    try:
        value = json.loads(content, object_pairs_hook=unique_object)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise Incomplete("observer input is not valid JSON") from error
    need(type(value) is dict, "observer input must be a JSON object")
    return value


def terminate_group(process: subprocess.Popen) -> None:
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def read_collector_output(process: subprocess.Popen, deadline: float, output_limit: int) -> bytes:
    stdout = bytearray()
    size = 0
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        while selector.get_map():
            remaining = deadline - time.monotonic()
            need(remaining > 0, "collector timeout; no qualification result")
            for key, _ in selector.select(min(remaining, 0.1)):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                size += len(chunk)
                need(size <= output_limit, "collector exceeded the output limit; no qualification result")
                if key.data == "stdout":
                    stdout.extend(chunk)
            if process.poll() is not None:
                # A collector must not leave a server or pipe-owning child behind.
                terminate_group(process)
    return bytes(stdout)


def run_collector(command: list[str], root: Path, timeout: float, output_limit: int) -> bytes:
    process = subprocess.Popen(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    deadline = time.monotonic() + timeout
    try:
        output = read_collector_output(process, deadline, output_limit)
        remaining = deadline - time.monotonic()
        need(remaining > 0, "collector timeout; no qualification result")
        try:
            code = process.wait(timeout=remaining)
        except subprocess.TimeoutExpired as error:
            raise Incomplete("collector timeout; no qualification result") from error
        need(code == 0, f"collector exited {code}; repair its execution before judging the requirement")
        return output
    finally:
        terminate_group(process)
        process.wait()
        process.stdout.close()
        process.stderr.close()


def collect(root: Path, requirement: str, nonce: str, timeout: float, output_limit: int) -> dict:
    collector = root / "proof/collect.py"
    need(collector.is_file(), "real consumer collector proof/collect.py is not implemented; framework integration remains unverified")
    dependency = sdk_checkout(root)
    framework_identity = checkout_identity(root, FRAMEWORK_INPUTS)
    sdk_identity = checkout_identity(dependency)
    command = [sys.executable, "-B", str(collector), requirement, "--nonce", nonce,
               "--framework-identity", framework_identity, "--sdk-identity", sdk_identity]
    stdout = run_collector(command, root, timeout, output_limit)

    need(checkout_identity(root, FRAMEWORK_INPUTS) == framework_identity and
         checkout_identity(dependency) == sdk_identity,
         "declared checkout inputs changed during collection")
    report = fields(parse_json(bytes(stdout)), "schema kind requirement nonce framework_identity sdk_identity observation", "collector report")
    need(type(report["schema"]) is int and report["schema"] == 1, "unsupported collector report schema")
    need(report["kind"] == "framework", "collector did not report a framework observation")
    need(report["requirement"] == requirement, "collector reported a different requirement")
    need(report["nonce"] == nonce, "collector returned a stale or unrelated observation")
    need(report["framework_identity"] == framework_identity and report["sdk_identity"] == sdk_identity,
         "collector did not identify the current consumed checkouts")
    if requirement == "LCT-001":
        observation = fields(report["observation"], "ownership_review", "ownership observation")
        review = fields(observation["ownership_review"], "source_identity", "ownership review")
        need(review["source_identity"] == sdk_identity, "ownership observation does not identify the captured SDK inputs")
    return report["observation"]


def execute(requirement: str, root: Path, *, timeout: float = TIMEOUT_SECONDS, output_limit: int = OUTPUT_LIMIT) -> int:
    kind = "unselected"
    try:
        need(requirement in OBSERVERS, "unknown integration requirement")
        with (root / "live_contract_checks/subject.json").open("rb") as handle:
            content = handle.read(OUTPUT_LIMIT + 1)
        need(len(content) <= OUTPUT_LIMIT, "observer subject exceeds the input limit")
        subject = fields(parse_json(content), "schema kind", "observer subject")
        need(type(subject["schema"]) is int and subject["schema"] == 1, "unsupported observer subject schema")
        kind = subject["kind"]
        need(kind in ("framework", "observer-control"), "unknown observer subject kind")
        if kind == "observer-control":
            fields(subject, "requirement observation", "observer control")
            need(subject["requirement"] == requirement, "control belongs to another requirement")
            observation = subject["observation"]
        else:
            observation = collect(root, requirement, secrets.token_hex(16), timeout, output_limit)
        evaluate(requirement, observation)
        need(kind == "framework", "observer-control satisfies the assertions but cannot qualify the framework")
    except Violation as error:
        print(f"{requirement}: {kind}: violation: {json.dumps(str(error))}")
        print(f"sudus: {requirement}: fail")
        return 1
    except (Incomplete, OSError) as error:
        print(f"{requirement}: unverified: {json.dumps(str(error))}")
        return 3
    print(f"sudus: {requirement}: pass")
    return 0


def interrupted(signum, frame):
    raise Incomplete(f"observer interrupted by signal {signum}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("requirement", choices=tuple(OBSERVERS))
    arguments = parser.parse_args()
    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        return execute(arguments.requirement, Path(__file__).resolve().parents[1])
    finally:
        signal.signal(signal.SIGTERM, previous)


if __name__ == "__main__":
    raise SystemExit(main())
