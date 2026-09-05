import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock


_LOCAL_ROOT = Path(__file__).resolve().parents[2]
_RUNNER_PATH = _LOCAL_ROOT / "scripts" / "gate-runner.py"


def _load_runner():
    spec = importlib.util.spec_from_file_location("suprnova_gate_runner", _RUNNER_PATH)
    if spec is None or spec.loader is None:  # pragma: no cover
        raise RuntimeError("failed to load gate runner module")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class RunnerDataTypeTests(unittest.TestCase):
    def test_outcome_values_and_step_shape_are_binding(self):
        runner = _load_runner()

        self.assertEqual(
            [outcome.value for outcome in runner.Outcome],
            [
                "pass",
                "fail",
                "timeout",
                "environment",
                "interrupted",
                "leak-detected",
            ],
        )
        step = runner.Step(
            id="probe",
            name="Probe",
            tiers=("default", "full"),
            argv=("python3", "probe.py"),
            timeout_seconds=12,
            category="code",
            capabilities=("python3",),
        )
        self.assertEqual(step.id, "probe")
        self.assertEqual(step.tiers, ("default", "full"))
        with self.assertRaises(Exception):
            step.id = "changed"


class StepRegistryTests(unittest.TestCase):
    def test_registry_matches_the_binding_step_contract(self):
        registry_path = _LOCAL_ROOT / "scripts" / "gate-steps.json"
        with registry_path.open(encoding="utf-8") as handle:
            registry = json.load(handle)

        expected = [
            ("fmt", ["default", "full"], 120, ["cargo", "fmt", "--all", "--check"]),
            (
                "doc-references",
                ["default", "full", "docs"],
                120,
                ["scripts/check-doc-references.sh"],
            ),
            (
                "prose-dashes",
                ["default", "full", "docs"],
                120,
                ["scripts/check-prose-dashes.sh"],
            ),
            (
                "workspace-clippy",
                ["default", "full"],
                1200,
                ["cargo", "clippy", "--workspace", "--all-targets"],
            ),
            (
                "json-rustdoc",
                ["default", "full"],
                900,
                [
                    "env",
                    "RUSTC_BOOTSTRAP=1",
                    "cargo",
                    "rustdoc",
                    "-p",
                    "suprnova",
                    "--lib",
                    "--",
                    "-Z",
                    "unstable-options",
                    "--output-format",
                    "json",
                ],
            ),
            (
                "json-rustdoc-live",
                ["default", "full"],
                900,
                [
                    "env",
                    "RUSTC_BOOTSTRAP=1",
                    "cargo",
                    "rustdoc",
                    "-p",
                    "suprnova-live",
                    "--lib",
                    "--",
                    "-Z",
                    "unstable-options",
                    "--output-format",
                    "json",
                ],
            ),
            (
                "json-rustdoc-macros",
                ["default", "full"],
                900,
                [
                    "env",
                    "RUSTC_BOOTSTRAP=1",
                    "cargo",
                    "rustdoc",
                    "-p",
                    "suprnova-macros",
                    "--lib",
                    "--",
                    "-Z",
                    "unstable-options",
                    "--output-format",
                    "json",
                ],
            ),
            (
                "json-rustdoc-test-support",
                ["default", "full"],
                900,
                [
                    "env",
                    "RUSTC_BOOTSTRAP=1",
                    "cargo",
                    "rustdoc",
                    "-p",
                    "suprnova-live-test-support",
                    "--lib",
                    "--",
                    "-Z",
                    "unstable-options",
                    "--output-format",
                    "json",
                ],
            ),
            (
                "workspace-tests",
                ["default", "full"],
                1800,
                ["cargo", "test", "--workspace", "--no-fail-fast"],
            ),
            (
                "magnetar-all-feature-tests",
                ["default", "full"],
                1800,
                [
                    "cargo",
                    "test",
                    "-p",
                    "suprnova-magnetar",
                    "--all-features",
                    "--tests",
                    "--no-fail-fast",
                ],
            ),
            (
                "postgres-tests",
                ["default", "full"],
                600,
                ["scripts/check-postgres.sh"],
            ),
            (
                "scaffold-tests",
                ["default", "full"],
                2400,
                [
                    "cargo",
                    "test",
                    "-p",
                    "suprnova-cli",
                    "--test",
                    "scaffold_snapshot",
                    "--test",
                    "live_generated_app",
                    "--",
                    "--ignored",
                ],
            ),
            (
                "translation-lock",
                ["docs", "full"],
                300,
                ["scripts/check-manual-translations.sh"],
            ),
            (
                "manual-structure",
                ["docs", "full"],
                120,
                ["python3", "scripts/check-manual-structure.py"],
            ),
            (
                "all-feature-clippy",
                ["full"],
                1800,
                [
                    "cargo",
                    "clippy",
                    "-p",
                    "suprnova",
                    "--all-targets",
                    "--features",
                    "otel,broadcasting-fanout,vector-pinecone,filesystem-azure,filesystem-gcs",
                ],
            ),
            (
                "mysql-relations",
                ["full"],
                600,
                ["scripts/check-mysql.sh"],
            ),
            (
                "magnetar-live-databases",
                ["full"],
                1200,
                ["scripts/check-magnetar-live.sh"],
            ),
            ("msrv", ["full"], 1800, ["scripts/check-msrv.sh"]),
            (
                "feature-matrix",
                ["full"],
                2700,
                ["scripts/check-feature-matrix.sh"],
            ),
            (
                "feature-pinecone-tests",
                ["full"],
                1800,
                [
                    "cargo",
                    "test",
                    "-p",
                    "suprnova",
                    "--features",
                    "vector-pinecone",
                    "--no-fail-fast",
                ],
            ),
            (
                "feature-fanout-tests",
                ["full"],
                1800,
                [
                    "cargo",
                    "test",
                    "-p",
                    "suprnova",
                    "--features",
                    "broadcasting-fanout",
                    "--no-fail-fast",
                ],
            ),
            ("audit", ["full"], 600, ["scripts/check-audit.sh"]),
            (
                "downstream-security",
                ["full"],
                1200,
                ["scripts/check-downstream-dependencies.sh"],
            ),
            (
                "release-smoke",
                ["full"],
                1200,
                ["scripts/tests/release-normal-smoke.sh"],
            ),
        ]
        actual = [
            (step["id"], step["tiers"], step["timeout_seconds"], step["argv"])
            for step in registry["steps"]
        ]
        self.assertEqual(actual, expected)

        self.assertEqual(
            registry["documentation_allowlist"],
            ["manual/**", ".manual-translations.lock", "README.md", "CHANGELOG.md", "LICENSE"],
        )
        self.assertEqual(
            registry["registered_files"],
            [
                {
                    "path": "README.md",
                    "checks": ["utf8", "doc-references", "prose-dashes"],
                },
                {
                    "path": "CHANGELOG.md",
                    "checks": ["utf8", "doc-references", "prose-dashes"],
                },
                {
                    "path": "LICENSE",
                    "checks": ["utf8", "doc-references", "prose-dashes"],
                },
            ],
        )

    def test_registry_rejects_executable_paths_that_can_escape_the_repo(self):
        runner = _load_runner()
        original = json.loads(
            (_LOCAL_ROOT / "scripts/gate-steps.json").read_text(encoding="utf-8")
        )
        invalid_paths = [
            "/tmp/suprnova-gate-helper",
            "../suprnova-gate-helper",
            "scripts\\suprnova-gate-helper",
            "~/suprnova-gate-helper",
        ]

        for executable in invalid_paths:
            with self.subTest(executable=executable):
                workspace = Path(
                    tempfile.mkdtemp(prefix="gate-registry-path-test-")
                )
                self.addCleanup(
                    lambda path=workspace: shutil.rmtree(path, ignore_errors=True)
                )
                registry = json.loads(json.dumps(original))
                registry["steps"][0]["argv"][0] = executable
                path = workspace / "scripts/gate-steps.json"
                path.parent.mkdir(parents=True)
                path.write_text(json.dumps(registry), encoding="utf-8")

                with self.assertRaisesRegex(
                    EnvironmentError, "invalid gate step executable path"
                ):
                    runner.load_registry(workspace)




class RunStepTests(unittest.TestCase):
    def setUp(self):
        self.runner = _load_runner()
        self.workspace = Path(tempfile.mkdtemp(prefix="gate-runner-test-"))
        self.addCleanup(lambda: shutil.rmtree(self.workspace, ignore_errors=True))
        self.run_dir = self.workspace / "run"
        self.run_dir.mkdir()

    def _script(self, name, source):
        path = self.workspace / name
        path.write_text(source, encoding="utf-8")
        return path

    def _context(self, run_id="test-run", **overrides):
        values = {
            "repo": self.workspace,
            "run_id": run_id,
            "run_dir": self.run_dir,
            "tier": "default",
            "env": dict(os.environ),
            "termination_grace_seconds": 0.2,
            "interrupt_event": threading.Event(),
            "container_cli": None,
        }
        values.update(overrides)
        return self.runner.RunContext(**values)

    def _step(self, script, *, timeout=5, capabilities=()):
        return self.runner.Step(
            id="probe",
            name="Probe",
            tiers=("default",),
            argv=(sys.executable, str(script)),
            timeout_seconds=timeout,
            category="code",
            capabilities=capabilities,
        )

    def test_leak_details_are_redacted_before_terminal_output(self):
        secret = "leaked-generated-password"
        result = self.runner.StepResult(
            step="probe",
            tier="default",
            outcome=self.runner.Outcome.LEAK_DETECTED,
            seconds=0.1,
            exit_code=1,
            argv=("scripts/check-magnetar-live.sh",),
            log_path=str(self.run_dir / "probe.log"),
            started=True,
            leaks=(
                {
                    "kind": "process",
                    "command": f"mariadb-admin --password={secret}",
                },
            ),
        )
        output = io.StringIO()

        with contextlib.redirect_stdout(output):
            self.runner._print_step_result(result)

        self.assertNotIn(secret, output.getvalue())
        self.assertIn("--password=[REDACTED]", output.getvalue())

    def _ledger(self):
        path = self.run_dir / "results.jsonl"
        if not path.exists():
            return []
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]

    def _wait_gone(self, pid):
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if not Path(f"/proc/{pid}").exists():
                return
            time.sleep(0.05)
        self.fail(f"process {pid} survived gate cleanup")

    def test_pass_records_duration_log_run_id_and_exactly_one_result(self):
        script = self._script(
            "pass.py",
            "import os\nprint('run=' + os.environ['SUPRNOVA_GATE_RUN_ID'])\n",
        )

        result = self.runner.run_step(self._step(script), self._context())

        self.assertEqual(result.outcome, self.runner.Outcome.PASS)
        self.assertEqual(result.exit_code, 0)
        self.assertGreaterEqual(result.seconds, 0)
        self.assertIn("run=test-run", Path(result.log_path).read_text(encoding="utf-8"))
        ledger = self._ledger()
        self.assertEqual(len(ledger), 1)
        self.assertEqual(ledger[0]["outcome"], "pass")
        self.assertEqual(ledger[0]["argv"], [sys.executable, str(script)])

    def test_nonzero_is_fail_and_same_group_grandchild_is_reaped(self):
        pid_file = self.workspace / "grandchild.pid"
        script = self._script(
            "fail.py",
            "import os, subprocess, sys, time\n"
            "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
            "open(os.environ['PID_FILE'], 'w').write(str(child.pid))\n"
            "time.sleep(0.1)\n"
            "raise SystemExit(7)\n",
        )
        context = self._context(env={**os.environ, "PID_FILE": str(pid_file)})

        result = self.runner.run_step(self._step(script), context)

        self.assertEqual(result.outcome, self.runner.Outcome.FAIL)
        self.assertEqual(result.exit_code, 7)
        self._wait_gone(int(pid_file.read_text(encoding="utf-8")))
        self.assertEqual(len(self._ledger()), 1)

    def test_timeout_kills_child_and_grandchild(self):
        parent_pid = self.workspace / "parent.pid"
        child_pid = self.workspace / "child.pid"
        script = self._script(
            "timeout.py",
            "import os, subprocess, sys, time\n"
            "open(os.environ['PARENT_PID'], 'w').write(str(os.getpid()))\n"
            "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
            "open(os.environ['CHILD_PID'], 'w').write(str(child.pid))\n"
            "time.sleep(30)\n",
        )
        context = self._context(
            env={
                **os.environ,
                "PARENT_PID": str(parent_pid),
                "CHILD_PID": str(child_pid),
            }
        )

        result = self.runner.run_step(self._step(script, timeout=1), context)

        self.assertEqual(result.outcome, self.runner.Outcome.TIMEOUT)
        self._wait_gone(int(parent_pid.read_text(encoding="utf-8")))
        self._wait_gone(int(child_pid.read_text(encoding="utf-8")))
        self.assertEqual(len(self._ledger()), 1)

    def test_interrupt_records_interrupted_and_cleans_group(self):
        pid_file = self.workspace / "interrupt.pid"
        script = self._script(
            "interrupt.py",
            "import os, time\n"
            "open(os.environ['PID_FILE'], 'w').write(str(os.getpid()))\n"
            "time.sleep(30)\n",
        )
        context = self._context(env={**os.environ, "PID_FILE": str(pid_file)})
        timer = threading.Timer(0.2, context.interrupt_event.set)
        timer.start()
        self.addCleanup(timer.cancel)

        result = self.runner.run_step(self._step(script), context)

        self.assertEqual(result.outcome, self.runner.Outcome.INTERRUPTED)
        self._wait_gone(int(pid_file.read_text(encoding="utf-8")))
        self.assertEqual(len(self._ledger()), 1)

    def test_setsid_escape_is_detected_identified_and_cleaned(self):
        escaped_pid = self.workspace / "escaped.pid"
        script = self._script(
            "escape.py",
            "import os, subprocess, sys, time\n"
            "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)\n"
            "open(os.environ['ESCAPED_PID'], 'w').write(str(child.pid))\n"
            "time.sleep(0.1)\n",
        )
        context = self._context(env={**os.environ, "ESCAPED_PID": str(escaped_pid)})

        result = self.runner.run_step(self._step(script), context)

        pid = int(escaped_pid.read_text(encoding="utf-8"))
        self.assertEqual(result.outcome, self.runner.Outcome.LEAK_DETECTED)
        self.assertTrue(any(leak["kind"] == "process" and leak["pid"] == pid for leak in result.leaks))
        self._wait_gone(pid)
        self.assertEqual(len(self._ledger()), 1)

    def test_escaped_child_holding_log_descriptor_cannot_block_runner_eof(self):
        escaped_pid = self.workspace / "holder.pid"
        script = self._script(
            "holder.py",
            "import os, subprocess, sys, time\n"
            "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)\n"
            "open(os.environ['ESCAPED_PID'], 'w').write(str(child.pid))\n"
            "print('parent done', flush=True)\n",
        )
        context = self._context(env={**os.environ, "ESCAPED_PID": str(escaped_pid)})

        started = time.monotonic()
        result = self.runner.run_step(self._step(script), context)

        self.assertLess(time.monotonic() - started, 3)
        self.assertEqual(result.outcome, self.runner.Outcome.LEAK_DETECTED)
        self._wait_gone(int(escaped_pid.read_text(encoding="utf-8")))

    def test_exact_run_label_container_is_removed_and_other_run_is_untouched(self):
        state = self.workspace / "containers.json"
        calls = self.workspace / "docker-calls.jsonl"
        state.write_text(
            json.dumps(
                [
                    {"id": "ours", "image": "postgres:test", "run": "test-run"},
                    {"id": "theirs", "image": "mysql:test", "run": "other-run"},
                ]
            ),
            encoding="utf-8",
        )
        docker = self._script(
            "fake-docker.py",
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "state_path = os.environ['FAKE_DOCKER_STATE']\n"
            "calls_path = os.environ['FAKE_DOCKER_CALLS']\n"
            "with open(calls_path, 'a') as f: f.write(json.dumps(sys.argv[1:]) + chr(10))\n"
            "with open(state_path) as f: state = json.load(f)\n"
            "if sys.argv[1] == 'ps':\n"
            "    label = sys.argv[sys.argv.index('--filter') + 1].split('=', 2)[2]\n"
            "    for item in state:\n"
            "        if item['run'] == label: print(item['id'] + chr(9) + item['image'])\n"
            "elif sys.argv[1] == 'rm':\n"
            "    removed = set(sys.argv[3:])\n"
            "    with open(state_path, 'w') as f: json.dump([x for x in state if x['id'] not in removed], f)\n",
        )
        docker.chmod(0o755)
        script = self._script("container_parent.py", "print('helper trap skipped')\n")
        env = {
            **os.environ,
            "FAKE_DOCKER_STATE": str(state),
            "FAKE_DOCKER_CALLS": str(calls),
        }
        context = self._context(env=env, container_cli=(sys.executable, str(docker)))

        result = self.runner.run_step(self._step(script), context)

        self.assertEqual(
            result.outcome, self.runner.Outcome.LEAK_DETECTED, msg=result.message
        )
        self.assertIn(
            {"kind": "container", "id": "ours", "image": "postgres:test"},
            result.leaks,
        )
        remaining = json.loads(state.read_text(encoding="utf-8"))
        self.assertEqual([item["id"] for item in remaining], ["theirs"])
        invocations = [
            json.loads(line) for line in calls.read_text(encoding="utf-8").splitlines()
        ]
        self.assertIn(
            [
                "ps",
                "-a",
                "--filter",
                "label=suprnova-gate-run=test-run",
                "--format",
                "{{.ID}}" + chr(9) + "{{.Image}}",
            ],
            invocations,
        )

    def test_missing_capability_is_environment_and_starts_nothing(self):
        marker = self.workspace / "started"
        script = self._script(
            "must-not-start.py",
            "from pathlib import Path\nPath(" + repr(str(marker)) + ").write_text('bad')\n",
        )

        result = self.runner.run_step(
            self._step(script, capabilities=("definitely-missing-suprnova-command",)),
            self._context(),
        )

        self.assertEqual(result.outcome, self.runner.Outcome.ENVIRONMENT)
        self.assertFalse(result.started)
        self.assertFalse(marker.exists())
        self.assertEqual(self._ledger(), [])


class RunnerCliTests(unittest.TestCase):
    def setUp(self):
        self.workspace = Path(tempfile.mkdtemp(prefix="gate-cli-test-"))
        self.addCleanup(lambda: shutil.rmtree(self.workspace, ignore_errors=True))
        self.source = self.workspace / "source"
        self.repo = self.workspace / "public"
        self.bin_dir = self.workspace / "bin"
        self.bin_dir.mkdir()
        rustc = self.bin_dir / "rustc"
        rustc.write_text("#!/bin/sh\nprintf 'rustc 1.94.0 (runner-test)\\n'\n", encoding="utf-8")
        rustc.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": str(self.bin_dir) + os.pathsep + os.environ.get("PATH", ""),
        }
        self.local_assets = [
            "scripts/gate-assets.json",
            "scripts/gate-runner.py",
            "scripts/gate-steps.json",
            "scripts/probe-helper",
            "scripts/gate.sh",
            "scripts/install-gate.py",
            ".githooks/pre-push",
            ".cargo/audit.toml",
        ]
        self.local_capabilities = ["bash", "cargo", "docker", "git", "python3"]
        self._init_repo(self.source, "local/gate-infra")
        self._init_repo(self.repo, "main")
        self._seed_public_repo()
        self._seed_local_source()
        self._install()

    def _init_repo(self, path, branch):
        path.mkdir(parents=True)
        subprocess.run(["git", "init", "--quiet", "-b", branch], cwd=path, check=True)
        subprocess.run(["git", "config", "user.name", "Gate Test"], cwd=path, check=True)
        subprocess.run(["git", "config", "user.email", "gate@example.com"], cwd=path, check=True)

    def _seed_public_repo(self):
        newline = chr(10)
        (self.repo / ".gitignore").write_text(
            newline.join(["/scripts/", "/.githooks/", "/.cargo/", ""]),
            encoding="utf-8",
        )
        (self.repo / "Cargo.toml").write_text(
            newline.join(["[workspace]", "members = []", ""]), encoding="utf-8"
        )
        (self.repo / "Cargo.lock").write_text(
            "version = 4" + newline, encoding="utf-8"
        )
        (self.repo / "rust-toolchain.toml").write_text(
            newline.join(['[toolchain]', 'channel = "1.94.0"', ""]),
            encoding="utf-8",
        )
        (self.repo / "probe.py").write_text(
            newline.join(
                [
                    "import os, subprocess, sys, time",
                    "start = os.environ.get('START_FILE')",
                    "if start:",
                    "    with open(start, 'a') as handle: handle.write('start' + chr(10))",
                    "pid_file = os.environ.get('PID_FILE')",
                    "if pid_file:",
                    "    open(pid_file, 'w').write(str(os.getpid()))",
                    "mode = os.environ.get('PROBE_MODE', 'pass')",
                    "if mode == 'fail':",
                    "    print('classified failure output')",
                    "    raise SystemExit(9)",
                    "if mode == 'secret-fail':",
                    "    print(os.environ['SECRET_URL'])",
                    "    print('POSTGRES_PASSWORD=' + os.environ['POSTGRES_SECRET'])",
                    "    print('password: ' + os.environ['GENERIC_SECRET'])",
                    "    raise SystemExit(9)",
                    "if mode == 'escape':",
                    "    child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)",
                    "    open(os.environ['ESCAPED_PID'], 'w').write(str(child.pid))",
                    "if mode in ('sleep', 'escape'):",
                    "    time.sleep(30)",
                    "print('probe passed')",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        subprocess.run(["git", "add", "."], cwd=self.repo, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "public"], cwd=self.repo, check=True)

    def _registry(self, *, capabilities=None, retention=None):
        return {
            "schema": 1,
            "retention": retention or {"max_runs": 50, "max_age_days": 30},
            "documentation_allowlist": [
                "manual/**",
                ".manual-translations.lock",
                "README.md",
                "CHANGELOG.md",
                "LICENSE",
            ],
            "registered_files": [
                {
                    "path": "README.md",
                    "checks": ["utf8", "doc-references", "prose-dashes"],
                },
                {
                    "path": "CHANGELOG.md",
                    "checks": ["utf8", "doc-references", "prose-dashes"],
                },
                {
                    "path": "LICENSE",
                    "checks": ["utf8", "doc-references", "prose-dashes"],
                },
            ],
            "steps": [
                {
                    "id": "probe",
                    "name": "Probe",
                    "tiers": ["default", "full"],
                    "argv": ["scripts/probe-helper"],
                    "timeout_seconds": 5,
                    "category": "code",
                    "capabilities": capabilities or [],
                }
            ],
        }

    def _seed_local_source(self):
        for relative in self.local_assets:
            path = self.source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            if relative == "scripts/gate-runner.py":
                shutil.copy2(_RUNNER_PATH, path)
            elif relative == "scripts/gate-steps.json":
                path.write_text(
                    json.dumps(self._registry(), indent=2) + chr(10), encoding="utf-8"
                )
            elif relative == "scripts/gate-assets.json":
                path.write_text(
                    json.dumps(
                        {
                            "schema": 1,
                            "branch": "local/gate-infra",
                            "assets": self.local_assets,
                            "capabilities": self.local_capabilities,
                        },
                        indent=2,
                    )
                    + chr(10),
                    encoding="utf-8",
                )
            elif relative == "scripts/probe-helper":
                path.write_text(
                    "#!/usr/bin/env bash\nexec python3 probe.py\n", encoding="utf-8"
                )
                path.chmod(0o755)
            else:
                path.write_text(
                    f"local asset {relative}" + chr(10), encoding="utf-8"
                )
            if relative == ".githooks/pre-push":
                path.chmod(0o755)
        subprocess.run(["git", "add", "-f", *self.local_assets], cwd=self.source, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "tooling"], cwd=self.source, check=True)

    def _install(self):
        self.local_commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=self.source, text=True
        ).strip()
        subprocess.run(
            [
                "git",
                "fetch",
                "--quiet",
                str(self.source),
                "local/gate-infra",
            ],
            cwd=self.repo,
            check=True,
        )
        manifest = json.loads(
            (self.source / "scripts/gate-assets.json").read_text(encoding="utf-8")
        )
        hashes = {}
        for relative in manifest["assets"]:
            source = self.source / relative
            destination = self.repo / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            hashes[relative] = hashlib.sha256(destination.read_bytes()).hexdigest()
        record = {
            "schema": 2,
            "branch": "local/gate-infra",
            "commit": self.local_commit,
            "assets": hashes,
            "capabilities": manifest["capabilities"],
        }
        git_path = subprocess.check_output(
            ["git", "rev-parse", "--git-path", "suprnova-local-gate.json"],
            cwd=self.repo,
            text=True,
        ).strip()
        (self.repo / git_path).write_text(
            json.dumps(record) + chr(10), encoding="utf-8"
        )
        subprocess.run(["git", "config", "core.hooksPath", ".githooks"], cwd=self.repo, check=True)

    def _replace_registry(self, registry):
        path = self.source / "scripts/gate-steps.json"
        path.write_text(
            json.dumps(registry, indent=2) + chr(10), encoding="utf-8"
        )
        subprocess.run(["git", "add", "-f", "scripts/gate-steps.json"], cwd=self.source, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "registry fixture"], cwd=self.source, check=True)
        self._install()

    def _run(self, *args, env=None, timeout=20):
        return subprocess.run(
            [sys.executable, "scripts/gate-runner.py", *args],
            cwd=self.repo,
            env=env or self.env,
            capture_output=True,
            text=True,
            timeout=timeout,
        )

    def _stamp_path(self):
        relative = subprocess.check_output(
            ["git", "rev-parse", "--git-path", "suprnova-gate-pass"],
            cwd=self.repo,
            text=True,
        ).strip()
        return self.repo / relative

    def _run_dirs(self):
        relative = subprocess.check_output(
            ["git", "rev-parse", "--git-path", "suprnova-gate-runs"],
            cwd=self.repo,
            text=True,
        ).strip()
        root = self.repo / relative
        return sorted((path for path in root.iterdir() if path.is_dir()), key=lambda path: path.name) if root.exists() else []

    def _wait_for_file(self, path):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if path.exists() and path.read_text(encoding="utf-8"):
                return
            time.sleep(0.02)
        self.fail(f"timed out waiting for {path}")

    def _wait_gone(self, pid):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if not Path(f"/proc/{pid}").exists():
                return
            time.sleep(0.05)
        self.fail(f"process {pid} survived")

    def test_success_writes_summary_stamp_and_mandatory_verdict(self):
        completed = self._run()

        self.assertEqual(completed.returncode, 0, msg=completed.stderr)
        self.assertIn("STEP", completed.stdout)
        self.assertIn("OUTCOME", completed.stdout)
        self.assertIn("SECONDS", completed.stdout)
        self.assertIn("LOG", completed.stdout)
        self.assertRegex(
            completed.stdout, r"(?m)^probe\s+pass\s+[0-9.]+\s+.+/probe\.log$"
        )
        self.assertTrue(self._stamp_path().is_file())
        run_dirs = self._run_dirs()
        self.assertEqual(len(run_dirs), 1)
        summary = json.loads((run_dirs[0] / "summary.json").read_text(encoding="utf-8"))
        self.assertEqual(summary["outcome"], "pass")
        self.assertEqual(summary["tier"], "default")
        self.assertEqual(len((run_dirs[0] / "results.jsonl").read_text().splitlines()), 1)
        self.assertEqual(
            completed.stdout.strip().splitlines()[-1],
            f"GATE GREEN: default, tree {summary['tree']}, run {summary['run_id']}",
        )

    def test_direct_named_step_diagnosis_never_writes_stamp(self):
        completed = self._run("--step", "probe")

        self.assertEqual(completed.returncode, 0, msg=completed.stderr)
        summary = json.loads(
            (self._run_dirs()[0] / "summary.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            completed.stdout.strip().splitlines()[-1],
            f"GATE GREEN: default, tree {summary['tree']}, run {summary['run_id']}",
        )
        self.assertFalse(self._stamp_path().exists())

    def test_failure_prints_compact_diagnosis_without_retry_recommendation(self):
        completed = self._run(env={**self.env, "PROBE_MODE": "fail"})

        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0)
        self.assertTrue(completed.stdout.rstrip().endswith("GATE FAILED: fail"))
        self.assertRegex(
            completed.stdout, r"(?m)^probe\s+fail\s+[0-9.]+\s+.+/probe\.log$"
        )
        self.assertIn("classified failure output", output)
        self.assertIn("command:", output)
        self.assertIn("log:", output)
        self.assertIn("diagnose: python3 scripts/gate-runner.py --step probe", output)
        self.assertNotIn("retry", output.lower())
        self.assertNotIn("rerun the full", output.lower())
        self.assertFalse(self._stamp_path().exists())

    def test_failure_tail_redacts_database_credentials_and_keeps_logs_private(self):
        secrets_by_name = {
            "URL_SECRET": "url-userinfo-secret",
            "POSTGRES_SECRET": "postgres-password-secret",
            "GENERIC_SECRET": "generic-password-secret",
        }
        completed = self._run(
            env={
                **self.env,
                "PROBE_MODE": "secret-fail",
                "SECRET_URL": (
                    "postgres://postgres:"
                    + secrets_by_name["URL_SECRET"]
                    + "@127.0.0.1:5432/database"
                ),
                "POSTGRES_SECRET": secrets_by_name["POSTGRES_SECRET"],
                "GENERIC_SECRET": secrets_by_name["GENERIC_SECRET"],
            }
        )

        terminal = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0)
        for secret in secrets_by_name.values():
            self.assertNotIn(secret, terminal)
        self.assertIn("[REDACTED]", terminal)

        run_dir = self._run_dirs()[0]
        log = run_dir / "probe.log"
        raw_log = log.read_text(encoding="utf-8")
        for secret in secrets_by_name.values():
            self.assertIn(secret, raw_log)
        self.assertEqual(stat.S_IMODE(run_dir.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(log.stat().st_mode), 0o600)

    def test_missing_capability_preflight_starts_no_step(self):
        start_file = self.workspace / "starts"
        registry = self._registry(capabilities=["definitely-missing-suprnova-command"])
        self._replace_registry(registry)

        completed = self._run(env={**self.env, "START_FILE": str(start_file)})

        self.assertNotEqual(completed.returncode, 0)
        self.assertTrue(
            completed.stdout.rstrip().endswith("GATE FAILED: environment")
        )
        self.assertFalse(start_file.exists())
        run_dirs = self._run_dirs()
        self.assertEqual(len(run_dirs), 1)
        ledger = run_dirs[0] / "results.jsonl"
        self.assertFalse(ledger.exists() and ledger.read_text(encoding="utf-8"))

    def test_preflight_rejects_slash_executable_outside_manifest_closure(self):
        start_file = self.workspace / "unmanifested-start"
        helper = self.repo / "scripts/unmanifested-helper"
        helper.write_text(
            "#!/usr/bin/env bash\nprintf started > \"$START_FILE\"\n",
            encoding="utf-8",
        )
        helper.chmod(0o755)
        registry = self._registry()
        registry["steps"][0]["argv"] = ["scripts/unmanifested-helper"]
        self._replace_registry(registry)

        completed = self._run(env={**self.env, "START_FILE": str(start_file)})

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("manifest closure", completed.stdout + completed.stderr)
        self.assertFalse(start_file.exists())

    def test_preflight_rejects_manifested_helper_symlinked_outside_repo(self):
        start_file = self.workspace / "symlink-start"
        installed = self.repo / "scripts/probe-helper"
        outside = self.workspace / "outside-probe-helper"
        outside.write_bytes(installed.read_bytes())
        outside.chmod(0o755)
        installed.unlink()
        installed.symlink_to(outside)

        completed = self._run(env={**self.env, "START_FILE": str(start_file)})

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("path contains symlink", completed.stdout + completed.stderr)
        self.assertFalse(start_file.exists())

    def test_preflight_rejects_missing_interpreter_helper_argument(self):
        helper = "scripts/check-manual-structure.py"
        self.assertFalse((self.repo / helper).exists())
        registry = self._registry()
        registry["steps"][0]["argv"] = ["python3", helper]
        self._replace_registry(registry)

        completed = self._run()

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 2)
        self.assertIn("manifest closure", output)
        self.assertTrue(completed.stdout.rstrip().endswith("GATE FAILED: environment"))
        run_dir = self._run_dirs()[0]
        summary = json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))
        self.assertEqual(summary["outcome"], "environment")
        self.assertEqual(summary["results"], [])
        self.assertFalse((run_dir / "results.jsonl").exists())
        self.assertFalse(self._stamp_path().exists())

    def test_preflight_rejects_existing_unmanifested_interpreter_arguments(self):
        for root in ("scripts", ".githooks", ".cargo"):
            with self.subTest(root=root):
                start_file = self.workspace / f"{root.strip('.')}-argument-start"
                helper = f"{root}/unmanifested-argument.py"
                registry = self._registry()
                registry["steps"][0]["argv"] = ["python3", helper]
                self._replace_registry(registry)
                (self.repo / helper).write_text(
                    "from pathlib import Path\n"
                    "import os\n"
                    "Path(os.environ['START_FILE']).write_text('started')\n",
                    encoding="utf-8",
                )

                completed = self._run(
                    env={**self.env, "START_FILE": str(start_file)}
                )

                self.assertEqual(completed.returncode, 2)
                self.assertIn("manifest closure", completed.stdout + completed.stderr)
                self.assertFalse(start_file.exists())
                run_dir = self._run_dirs()[-1]
                self.assertFalse((run_dir / "results.jsonl").exists())

    def test_preflight_rejects_traversal_in_interpreter_argument(self):
        start_file = self.workspace / "traversal-argument-start"
        registry = self._registry()
        registry["steps"][0]["argv"] = ["python3", "scripts/../probe.py"]
        self._replace_registry(registry)

        completed = self._run(env={**self.env, "START_FILE": str(start_file)})

        self.assertEqual(completed.returncode, 2)
        self.assertIn("invalid local gate asset path", completed.stdout + completed.stderr)
        self.assertFalse(start_file.exists())
        run_dir = self._run_dirs()[0]
        self.assertFalse((run_dir / "results.jsonl").exists())

    def test_preflight_rejects_manifested_interpreter_argument_symlinked_outside_repo(
        self,
    ):
        start_file = self.workspace / "symlink-argument-start"
        registry = self._registry()
        registry["steps"][0]["argv"] = ["bash", "scripts/probe-helper"]
        self._replace_registry(registry)
        installed = self.repo / "scripts/probe-helper"
        outside = self.workspace / "outside-interpreter-helper"
        outside.write_bytes(installed.read_bytes())
        outside.chmod(0o755)
        installed.unlink()
        installed.symlink_to(outside)

        completed = self._run(env={**self.env, "START_FILE": str(start_file)})

        self.assertEqual(completed.returncode, 2)
        self.assertIn("path contains symlink", completed.stdout + completed.stderr)
        self.assertFalse(start_file.exists())
        run_dir = self._run_dirs()[0]
        self.assertFalse((run_dir / "results.jsonl").exists())

    def test_preflight_accepts_manifested_readable_interpreter_argument(self):
        start_file = self.workspace / "manifested-argument-start"
        registry = self._registry()
        registry["steps"][0]["argv"] = ["bash", "scripts/probe-helper"]
        self._replace_registry(registry)
        (self.source / "scripts/probe-helper").chmod(0o644)
        subprocess.run(
            ["git", "add", "-f", "scripts/probe-helper"],
            cwd=self.source,
            check=True,
        )
        subprocess.run(
            ["git", "commit", "--quiet", "-m", "readable helper fixture"],
            cwd=self.source,
            check=True,
        )
        self._install()

        completed = self._run(env={**self.env, "START_FILE": str(start_file)})

        self.assertEqual(completed.returncode, 0, msg=completed.stdout + completed.stderr)
        self.assertEqual(start_file.read_text(encoding="utf-8").splitlines(), ["start"])
        run_dir = self._run_dirs()[0]
        result = json.loads((run_dir / "results.jsonl").read_text().splitlines()[0])
        self.assertEqual(result["outcome"], "pass")

    def test_runner_rejects_installed_executable_mode_drift_before_step_start(self):
        runner = _load_runner()
        self.assertEqual(runner.verify_local_install(self.repo).commit, self.local_commit)

        (self.repo / ".githooks/pre-push").chmod(0o644)
        start_file = self.workspace / "mode-drift-start"

        with self.assertRaisesRegex(EnvironmentError, "executable mode drift"):
            runner.verify_local_install(self.repo)
        completed = self._run(env={**self.env, "START_FILE": str(start_file)})

        self.assertEqual(completed.returncode, 2)
        self.assertIn("executable mode drift", completed.stdout + completed.stderr)
        self.assertFalse(start_file.exists())


    def test_sigint_and_sigterm_each_record_interrupted_and_leave_no_process(self):
        for delivered_signal in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(delivered_signal=delivered_signal):
                pid_file = self.workspace / f"signal-{delivered_signal}.pid"
                process = subprocess.Popen(
                    [sys.executable, "scripts/gate-runner.py"],
                    cwd=self.repo,
                    env={**self.env, "PROBE_MODE": "sleep", "PID_FILE": str(pid_file)},
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                self._wait_for_file(pid_file)
                process.send_signal(delivered_signal)
                output, _ = process.communicate(timeout=10)

                self.assertNotEqual(process.returncode, 0)
                self.assertTrue(output.rstrip().endswith("GATE FAILED: interrupted"))
                self._wait_gone(int(pid_file.read_text(encoding="utf-8")))
                latest = self._run_dirs()[-1]
                result = json.loads((latest / "results.jsonl").read_text().splitlines()[-1])
                self.assertEqual(result["outcome"], "interrupted")
                self.assertFalse(self._stamp_path().exists())

    def test_sigkill_stale_run_is_cleaned_and_next_run_starts_no_step(self):
        pid_file = self.workspace / "stale.pid"
        start_file = self.workspace / "starts"
        process = subprocess.Popen(
            [sys.executable, "scripts/gate-runner.py"],
            cwd=self.repo,
            env={
                **self.env,
                "PROBE_MODE": "sleep",
                "PID_FILE": str(pid_file),
                "START_FILE": str(start_file),
            },
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self._wait_for_file(pid_file)
        process.kill()
        process.wait(timeout=5)

        recovered = self._run(
            env={
                **self.env,
                "PROBE_MODE": "pass",
                "START_FILE": str(start_file),
            }
        )

        self.assertNotEqual(recovered.returncode, 0)
        self.assertIn("stale run", (recovered.stdout + recovered.stderr).lower())
        self.assertTrue(
            recovered.stdout.rstrip().endswith("GATE FAILED: environment")
        )
        self.assertEqual(start_file.read_text(encoding="utf-8").splitlines(), ["start"])
        self._wait_gone(int(pid_file.read_text(encoding="utf-8")))

    def test_same_tree_outcome_flip_prints_environmental_fault_and_names_runs(self):
        first = self._run()
        self.assertEqual(first.returncode, 0)
        first_run = self._run_dirs()[-1].name

        second = self._run(env={**self.env, "PROBE_MODE": "fail"})

        second_run = self._run_dirs()[-1].name
        output = second.stdout + second.stderr
        self.assertIn("environmental fault", output.lower())
        self.assertIn(first_run, output)
        self.assertIn(second_run, output)
        self.assertFalse(self._stamp_path().exists())

    def test_result_history_is_bounded_by_count(self):
        registry = self._registry(retention={"max_runs": 2, "max_age_days": 30})
        self._replace_registry(registry)

        for _ in range(3):
            completed = self._run("--step", "probe")
            self.assertEqual(completed.returncode, 0, msg=completed.stderr)

        self.assertEqual(len(self._run_dirs()), 2)

    def test_capabilities_are_bound_into_steps_hash(self):
        runner = _load_runner()
        before = runner.compute_steps_hash(self.repo, "default")
        manifest_path = self.source / "scripts/gate-assets.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["capabilities"].append("new-capability")
        manifest_path.write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        subprocess.run(
            ["git", "add", "-f", "scripts/gate-assets.json"],
            cwd=self.source,
            check=True,
        )
        subprocess.run(
            ["git", "commit", "--quiet", "-m", "capability fixture"],
            cwd=self.source,
            check=True,
        )
        self._install()

        after = runner.compute_steps_hash(self.repo, "default")

        self.assertNotEqual(before, after)

    def test_comparison_only_scans_retained_prebloated_history(self):
        runner, registry = self._seed_retention_fixture()
        runs_root = runner._git_path(self.repo, "suprnova-gate-runs")
        for index in range(20):
            (runs_root / f"prebloated-{index:02d}").mkdir()
        observed_history_sizes = []
        original = runner._find_outcome_comparison

        def observe_retained_history(root, summary):
            observed_history_sizes.append(
                len([path for path in root.iterdir() if path.is_dir()])
            )
            return original(root, summary)

        with mock.patch.object(
            runner,
            "_find_outcome_comparison",
            side_effect=observe_retained_history,
        ):
            result = runner.execute_gate(
                self.repo,
                registry,
                tier="default",
                named_step="probe",
                env=self.env,
            )

        self.assertEqual(result, 0)
        self.assertEqual(observed_history_sizes, [2])
        self.assertEqual(len(self._run_dirs()), 2)

    def test_runner_rejects_symlinked_installed_asset_before_step_start(self):
        outside = self.workspace / "runner-symlink-target"
        asset = self.repo / ".cargo/audit.toml"
        outside.write_bytes(asset.read_bytes())
        asset.unlink()
        asset.symlink_to(outside)
        start_file = self.workspace / "symlink-start"

        completed = self._run(
            "--step",
            "probe",
            env={**self.env, "START_FILE": str(start_file)},
        )

        self.assertEqual(completed.returncode, 2)
        self.assertIn("symlink", (completed.stdout + completed.stderr).lower())
        self.assertFalse(start_file.exists())

    def test_interrupt_while_preflight_is_blocked_has_one_terminal_failure(self):
        runner, registry = self._seed_retention_fixture()
        start_file = self.workspace / "early-interrupt-start"
        preflight_entered = threading.Event()
        release_preflight = threading.Event()
        previous_handler_called = threading.Event()
        sender_errors = []
        original_handler = signal.signal(
            signal.SIGTERM,
            lambda _signum, _frame: previous_handler_called.set(),
        )

        def blocked_preflight(_repo, _steps, _env):
            preflight_entered.set()
            if not release_preflight.wait(timeout=5):
                raise RuntimeError("test preflight was not released")
            return None

        def send_interrupt():
            if not preflight_entered.wait(timeout=5):
                sender_errors.append("runner never entered mocked preflight")
                release_preflight.set()
                return
            os.kill(os.getpid(), signal.SIGTERM)
            time.sleep(0.05)
            release_preflight.set()

        sender = threading.Thread(target=send_interrupt)
        sender.start()
        terminal = io.StringIO()
        try:
            with (
                mock.patch.object(runner, "_preflight", side_effect=blocked_preflight),
                contextlib.redirect_stdout(terminal),
            ):
                result = runner.execute_gate(
                    self.repo,
                    registry,
                    tier="default",
                    named_step=None,
                    env={**self.env, "START_FILE": str(start_file)},
                )
        finally:
            release_preflight.set()
            sender.join(timeout=5)
            signal.signal(signal.SIGTERM, original_handler)

        self.assertFalse(sender.is_alive())
        self.assertEqual(sender_errors, [])
        self.assertFalse(previous_handler_called.is_set())
        self.assertEqual(result, 130)
        output = terminal.getvalue()
        self.assertEqual(output.count("GATE FAILED: interrupted"), 1)
        self.assertNotIn("GATE GREEN", output)
        self.assertFalse(start_file.exists())
        run_dirs = self._run_dirs()
        self.assertEqual(len(run_dirs), 2)
        summary_dirs = [
            directory for directory in run_dirs if (directory / "summary.json").exists()
        ]
        self.assertEqual(len(summary_dirs), 1)
        summary = json.loads(
            (summary_dirs[0] / "summary.json").read_text(encoding="utf-8")
        )
        self.assertEqual(summary["outcome"], "interrupted")
        self.assertEqual(summary["results"], [])
        self.assertFalse((summary_dirs[0] / "results.jsonl").exists())
        self.assertFalse(self._stamp_path().exists())
        self.assertFalse(runner._active_path(self.repo).exists())

    def _seed_retention_fixture(self):
        registry = self._registry(retention={"max_runs": 2, "max_age_days": 30})
        self._replace_registry(registry)
        runner = _load_runner()
        loaded = runner.load_registry(self.repo)
        runs_root = runner._git_path(self.repo, "suprnova-gate-runs")
        runs_root.mkdir(parents=True, exist_ok=True)
        for name in ("old-run-a", "old-run-b"):
            (runs_root / name).mkdir()
        return runner, loaded

    def test_tree_or_steps_hash_failure_still_prunes_run_history(self):
        runner, registry = self._seed_retention_fixture()

        with mock.patch.object(
            runner, "compute_steps_hash", side_effect=EnvironmentError("hash failure")
        ):
            result = runner.execute_gate(
                self.repo,
                registry,
                tier="default",
                named_step=None,
                env=self.env,
            )

        self.assertEqual(result, 2)
        self.assertEqual(len(self._run_dirs()), 2)

    def test_active_record_acquisition_race_still_prunes_run_history(self):
        runner, registry = self._seed_retention_fixture()

        with mock.patch.object(
            runner, "_acquire_active", side_effect=FileExistsError
        ):
            result = runner.execute_gate(
                self.repo,
                registry,
                tier="default",
                named_step=None,
                env=self.env,
            )

        self.assertEqual(result, 2)
        self.assertEqual(len(self._run_dirs()), 2)


class ShellAssetContractTests(unittest.TestCase):
    def setUp(self):
        self.workspace = Path(tempfile.mkdtemp(prefix="gate-shell-assets-test-"))
        self.addCleanup(lambda: shutil.rmtree(self.workspace, ignore_errors=True))
        self.repo = self.workspace / "repo"
        self.repo.mkdir()
        subprocess.run(
            ["git", "init", "--quiet", "-b", "main"], cwd=self.repo, check=True
        )
        for relative in [
            "scripts/gate.sh",
            ".githooks/pre-push",
            "scripts/check-postgres.sh",
            "scripts/check-mysql.sh",
            "scripts/check-magnetar-live.sh",
        ]:
            destination = self.repo / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(_LOCAL_ROOT / relative, destination)
            destination.chmod(0o755)

        pivot_target = (
            self.repo
            / "framework/tests/eloquent_relations_pivot_filters_postgres.rs"
        )
        pivot_target.parent.mkdir(parents=True, exist_ok=True)
        pivot_target.write_text("", encoding="utf-8")

        self.bin_dir = self.workspace / "bin"
        self.bin_dir.mkdir()
        self.python_args = self.workspace / "python-args"
        self.python_stdin = self.workspace / "python-stdin"
        self.docker_log = self.workspace / "docker.log"
        self.cargo_log = self.workspace / "cargo.log"
        self.magnetar_capture = self.workspace / "magnetar-capture"
        self._write_executable(
            self.bin_dir / "python3",
            "#!/usr/bin/env bash\n"
            "printf '%s\\n' \"$@\" > \"$FAKE_PYTHON_ARGS\"\n"
            "if [[ -n \"${FAKE_PYTHON_STDIN-}\" ]]; then\n"
            "    cat > \"$FAKE_PYTHON_STDIN\"\n"
            "fi\n",
        )
        self._write_executable(
            self.bin_dir / "docker",
            "#!/usr/bin/env bash\n"
            "printf '%s\\n' \"$*\" >> \"$FAKE_DOCKER_LOG\"\n"
            "case \"${1-}\" in\n"
            "    info|exec|rm|logs) exit 0 ;;\n"
            "    run) printf 'fake-container\\n' ;;\n"
            "    port)\n"
            "        if [[ \"${3-}\" == '5432/tcp' ]]; then\n"
            "            printf '127.0.0.1:15432\\n'\n"
            "        else\n"
            "            printf '127.0.0.1:13306\\n'\n"
            "        fi\n"
            "        ;;\n"
            "esac\n",
        )
        self._write_executable(
            self.bin_dir / "cargo",
            "#!/usr/bin/env bash\n"
            "printf '%s\\n' \"$*\" >> \"$FAKE_CARGO_LOG\"\n"
            "# A workflow selector names one exact test or a prefix shared by\n"
            "# two; print the harness lines the helpers assert on for each.\n"
            "for arg in \"$@\"; do\n"
            "    case \"$arg\" in\n"
            "        workflow::tests::test_mysql_)\n"
            "            printf 'test workflow::tests::test_mysql_one ... ok\\n'\n"
            "            printf 'test workflow::tests::test_mysql_two ... ok\\n'\n"
            "            printf 'test result: ok. 2 passed; 0 failed; 0 ignored\\n'\n"
            "            ;;\n"
            "        workflow::tests::*)\n"
            "            printf 'test %s ... ok\\n' \"$arg\"\n"
            "            printf 'test result: ok. 1 passed; 0 failed; 0 ignored\\n'\n"
            "            ;;\n"
            "        live_postgres)\n"
            "            printf 'test live_postgres_generation_ledger_advances_and_reads ... ok\\n'\n"
            "            printf 'test live_postgres_concurrent_advances_in_opposite_order_do_not_deadlock ... ok\\n'\n"
            "            printf 'test live_postgres_a_write_committed_during_a_cached_render_is_never_published_as_current ... ok\\n'\n"
            "            printf 'test result: ok. 3 passed; 0 failed; 0 ignored\\n'\n"
            "            ;;\n"
            "        live_mysql)\n"
            "            printf 'test live_mysql_generation_ledger_advances_and_reads ... ok\\n'\n"
            "            printf 'test live_mysql_concurrent_advances_in_opposite_order_do_not_deadlock ... ok\\n'\n"
            "            printf 'test live_mysql_a_write_committed_during_a_cached_render_is_never_published_as_current ... ok\\n'\n"
            "            printf 'test result: ok. 3 passed; 0 failed; 0 ignored\\n'\n"
            "            ;;\n"
            "    esac\n"
            "done\n",
        )
        self._write_executable(
            self.repo / "crates/suprnova-magnetar/scripts/gate.sh",
            "#!/usr/bin/env bash\n"
            "{\n"
            "    printf 'argv=%s\\n' \"$*\"\n"
            "    printf 'postgres=%s\\n' \"$MAGNETAR_POSTGRES_TEST_URL\"\n"
            "    printf 'mysql=%s\\n' \"$MAGNETAR_MYSQL_TEST_URL\"\n"
            "} > \"$MAGNETAR_CAPTURE\"\n",
        )
        self.env = {
            **os.environ,
            "PATH": str(self.bin_dir) + os.pathsep + os.environ.get("PATH", ""),
            "FAKE_PYTHON_ARGS": str(self.python_args),
            "FAKE_DOCKER_LOG": str(self.docker_log),
            "FAKE_CARGO_LOG": str(self.cargo_log),
            "MAGNETAR_CAPTURE": str(self.magnetar_capture),
        }

    @staticmethod
    def _write_executable(path, source):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8")
        path.chmod(0o755)

    def _run(self, relative, *args, input_text=None, env=None):
        return subprocess.run(
            [str(self.repo / relative), *args],
            cwd=self.repo,
            env=env or self.env,
            input=input_text,
            capture_output=True,
            text=True,
            timeout=20,
        )

    def test_wrapper_executes_only_supported_runner_invocations(self):
        default = self._run("scripts/gate.sh")
        self.assertEqual(default.returncode, 0, msg=default.stderr)
        self.assertEqual(
            self.python_args.read_text(encoding="utf-8").splitlines(),
            ["scripts/gate-runner.py"],
        )

        full = self._run("scripts/gate.sh", "--full")
        self.assertEqual(full.returncode, 0, msg=full.stderr)
        self.assertEqual(
            self.python_args.read_text(encoding="utf-8").splitlines(),
            ["scripts/gate-runner.py", "--full"],
        )

        for invalid in [("--step",), ("--full", "extra")]:
            with self.subTest(invalid=invalid):
                self.python_args.unlink(missing_ok=True)
                rejected = self._run("scripts/gate.sh", *invalid)
                self.assertEqual(rejected.returncode, 2)
                self.assertIn("usage: scripts/gate.sh [--full]", rejected.stderr)
                self.assertFalse(self.python_args.exists())

    def test_pre_push_executes_authorizer_with_remote_and_forwards_all_refs(self):
        updates = (
            f"refs/heads/main {'1' * 40} refs/heads/main {'0' * 40}\n"
            f"HEAD {'2' * 40} refs/heads/local/gate-infra {'0' * 40}\n"
        )
        env = {**self.env, "FAKE_PYTHON_STDIN": str(self.python_stdin)}

        completed = self._run(
            ".githooks/pre-push",
            "origin",
            "git@github.com:owner/repo.git",
            input_text=updates,
            env=env,
        )

        self.assertEqual(completed.returncode, 0, msg=completed.stderr)
        self.assertEqual(
            self.python_args.read_text(encoding="utf-8").splitlines(),
            [
                "scripts/gate-runner.py",
                "--authorize-push",
                "origin",
                "git@github.com:owner/repo.git",
            ],
        )
        self.assertEqual(self.python_stdin.read_text(encoding="utf-8"), updates)

    def test_database_helpers_reject_missing_run_identity_before_docker(self):
        for relative in [
            "scripts/check-postgres.sh",
            "scripts/check-mysql.sh",
            "scripts/check-magnetar-live.sh",
        ]:
            with self.subTest(relative=relative):
                self.docker_log.unlink(missing_ok=True)
                completed = self._run(relative)
                self.assertNotEqual(completed.returncode, 0)
                self.assertIn("SUPRNOVA_GATE_RUN_ID must be set", completed.stderr)
                self.assertFalse(self.docker_log.exists())

    def test_database_helpers_use_fake_containers_with_exact_run_label(self):
        expectations = {
            "scripts/check-postgres.sh": ["postgres:17-alpine"],
            "scripts/check-mysql.sh": ["mariadb:11-jammy"],
            "scripts/check-magnetar-live.sh": [
                "postgres:17-alpine",
                "mysql:8.4",
            ],
        }
        cargo_expectations = {
            "scripts/check-postgres.sh": [
                "--test eloquent_relations_pivot_filters_postgres --",
                "workflow::tests::test_claim_reclaims_expired_running_row",
            ],
            "scripts/check-mysql.sh": [
                "--test eloquent_mass_write_mysql --",
                "workflow::tests::test_mysql_",
            ],
            "scripts/check-magnetar-live.sh": [],
        }
        for relative, images in expectations.items():
            with self.subTest(relative=relative):
                self.docker_log.write_text("", encoding="utf-8")
                self.cargo_log.write_text("", encoding="utf-8")
                self.magnetar_capture.unlink(missing_ok=True)
                completed = self._run(
                    relative,
                    env={**self.env, "SUPRNOVA_GATE_RUN_ID": "behavior-test-run"},
                )
                self.assertEqual(
                    completed.returncode,
                    0,
                    msg=completed.stdout + completed.stderr,
                )
                docker_calls = self.docker_log.read_text(encoding="utf-8")
                self.assertIn(
                    "--label suprnova-gate-run=behavior-test-run", docker_calls
                )
                self.assertIn("rm -f", docker_calls)
                for image in images:
                    self.assertIn(image, docker_calls)
                cargo_calls = self.cargo_log.read_text(encoding="utf-8")
                for invocation in cargo_expectations[relative]:
                    self.assertIn(invocation, cargo_calls)

        magnetar_docker_calls = self.docker_log.read_text(
            encoding="utf-8"
        ).splitlines()
        magnetar_run_calls = [
            call for call in magnetar_docker_calls if call.startswith("run ")
        ]
        self.assertEqual(
            [call.rsplit(" ", 1)[-1] for call in magnetar_run_calls],
            ["postgres:17-alpine", "mysql:8.4"],
        )
        mysql_run = magnetar_run_calls[1]
        self.assertNotIn("mariadb:", mysql_run)
        self.assertNotIn("MARIADB_", mysql_run)
        self.assertIn("-e MYSQL_ROOT_PASSWORD=", mysql_run)
        self.assertIn("-e MYSQL_DATABASE=magnetar_test", mysql_run)
        self.assertIn("-e MYSQL_ROOT_HOST=%", mysql_run)
        self.assertIn("-p 127.0.0.1::3306", mysql_run)
        mysql_ready_calls = [
            call
            for call in magnetar_docker_calls
            if call.startswith("exec ") and "mysqladmin ping" in call
        ]
        self.assertEqual(len(mysql_ready_calls), 1)
        self.assertIn("--host=127.0.0.1", mysql_ready_calls[0])
        self.assertIn("--user=root", mysql_ready_calls[0])
        self.assertIn("--password=gate-", mysql_ready_calls[0])
        self.assertIn("--silent", mysql_ready_calls[0])
        self.assertFalse(
            any("mariadb-admin" in call for call in magnetar_docker_calls)
        )

        magnetar = self.magnetar_capture.read_text(encoding="utf-8").splitlines()
        self.assertEqual(magnetar[0], "argv=--live")
        self.assertTrue(magnetar[1].startswith("postgres=postgres://postgres:"))
        self.assertTrue(magnetar[2].startswith("mysql=mysql://root:"))
if __name__ == "__main__":
    unittest.main()
