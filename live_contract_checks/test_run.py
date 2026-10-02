"""Exercise the real runner with bounded, local collector processes."""

from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

from live_contract_checks.control_examples import example, violating_example
from live_contract_checks.run import execute


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="live-observer-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "live_contract_checks").mkdir()
        (self.root / "proof").mkdir()
        dependency = self.root / "sdk"
        dependency.mkdir()
        subprocess.run(["git", "init", "-q", str(dependency)], check=True)
        (dependency / "Cargo.toml").write_text('[workspace]\nmembers = []\n')
        (dependency / "source.rs").write_text("// dependency input\n")
        (self.root / "proof/sdk.json").write_text('{"schema": 1, "checkout": "sdk"}\n')
        self.subject({"schema": 1, "kind": "framework"})

    def subject(self, value):
        (self.root / "live_contract_checks/subject.json").write_text(json.dumps(value))

    def collect(self, source):
        (self.root / "proof/collect.py").write_text(source)

    def run_requirement(self, requirement="LCT-001", **options):
        output = io.StringIO()
        with redirect_stdout(output):
            code = execute(requirement, self.root, **options)
        return code, output.getvalue()

    def test_missing_real_collector_is_unverified_not_a_requirement_failure(self):
        code, output = self.run_requirement()
        self.assertEqual(code, 3)
        self.assertIn("unverified", output)
        self.assertNotIn("sudus: LCT-001:", output)

    def test_each_control_uses_the_real_assertion_and_records_a_failure(self):
        for requirement in ["LCT-001", "LCT-002", "LCT-003", "LCT-004", "LCT-005", "LCT-006"]:
            self.subject({"schema": 1, "kind": "observer-control", "requirement": requirement, "observation": violating_example(requirement)})
            with self.subTest(requirement=requirement):
                code, output = self.run_requirement(requirement)
                self.assertEqual(code, 1)
                self.assertIn(f"sudus: {requirement}: fail", output)
                self.assertIn("observer-control", output)

    def test_a_successful_control_can_never_qualify_the_framework(self):
        self.subject({"schema": 1, "kind": "observer-control", "requirement": "LCT-001", "observation": example("LCT-001")})
        code, output = self.run_requirement()
        self.assertEqual(code, 3)
        self.assertNotIn("sudus: LCT-001: pass", output)

    def test_incomplete_control_is_unverified_not_bound_as_a_violation(self):
        self.subject({"schema": 1, "kind": "observer-control", "requirement": "LCT-001", "observation": {}})
        code, output = self.run_requirement()
        self.assertEqual(code, 3)
        self.assertNotIn("sudus: LCT-001: fail", output)

    def test_collector_cannot_inject_a_result_line(self):
        self.collect("print('sudus: LCT-001: pass')\nraise SystemExit(1)\n")
        code, output = self.run_requirement()
        self.assertEqual(code, 3)
        self.assertNotIn("sudus: LCT-001: pass", output)

    def test_wrong_requirement_or_replayed_nonce_never_passes(self):
        for extra in [{"requirement": "LCT-002"}, {"nonce": "old-run"}, {"framework_identity": "old-inputs"}, {"sdk_identity": "old-inputs"}]:
            self.collect(self.collector_source(extra))
            with self.subTest(extra=extra):
                code, output = self.run_requirement()
                self.assertEqual(code, 3)
                self.assertNotIn("sudus: LCT-001: pass", output)

    def test_fresh_complete_collector_result_uses_the_observer(self):
        self.collect(self.collector_source({}))
        code, output = self.run_requirement()
        self.assertEqual(code, 0)
        self.assertIn("sudus: LCT-001: pass", output)
        self.collect(self.collector_source({"observation": violating_example("LCT-001")}))
        code, output = self.run_requirement()
        self.assertEqual(code, 1)
        self.assertIn("sudus: LCT-001: fail", output)

    def collector_source(self, overrides):
        # A test of the runner's wire protocol, in an isolated temporary root.
        # This collector is never installed in the real proof directory.
        return (
            "import json, sys\n"
            f"result = {repr({'schema': 1, 'kind': 'framework', 'requirement': 'LCT-001', 'observation': example('LCT-001')})}\n"
            "result['nonce'] = sys.argv[sys.argv.index('--nonce') + 1]\n"
            "result['framework_identity'] = sys.argv[sys.argv.index('--framework-identity') + 1]\n"
            "result['sdk_identity'] = sys.argv[sys.argv.index('--sdk-identity') + 1]\n"
            f"result.update({overrides!r})\n"
            "result['observation']['ownership_review']['source_identity'] = result['sdk_identity']\n"
            "print(json.dumps(result))\n"
        )

    def test_timeout_is_unverified_and_cannot_report_pass(self):
        self.collect("import time\ntime.sleep(30)\n")
        code, output = self.run_requirement(timeout=0.1)
        self.assertEqual(code, 3)
        self.assertIn("timeout", output)
        self.assertNotIn("sudus: LCT-001: pass", output)

    def test_output_limit_is_unverified(self):
        self.collect("print('x' * 20000)\n")
        code, output = self.run_requirement(output_limit=1024)
        self.assertEqual(code, 3)
        self.assertIn("output limit", output)
        self.assertNotIn("sudus: LCT-001: pass", output)

    def test_malformed_and_duplicate_key_subjects_never_pass(self):
        for content in ['{', '{"schema": 1, "kind": "framework", "kind": "observer-control"}']:
            (self.root / "live_contract_checks/subject.json").write_text(content)
            with self.subTest(content=content):
                code, output = self.run_requirement()
                self.assertEqual(code, 3)
                self.assertNotIn("sudus: LCT-001:", output)

    def test_collector_exit_zero_with_no_evidence_never_passes(self):
        self.collect("pass\n")
        code, output = self.run_requirement()
        self.assertEqual(code, 3)
        self.assertNotIn("sudus: LCT-001: pass", output)

    def test_dependency_change_during_collection_invalidates_the_result(self):
        self.collect("from pathlib import Path\nPath('sdk/source.rs').write_text('// changed during collection\\n')\n" + self.collector_source({}))
        code, output = self.run_requirement()
        self.assertEqual(code, 3)
        self.assertIn("changed during", output)
        self.assertNotIn("sudus: LCT-001: pass", output)

    def test_unconfigured_dependency_cannot_produce_compatibility_evidence(self):
        (self.root / "proof/sdk.json").write_text('{"schema": 1, "checkout": null}\n')
        self.collect(self.collector_source({}))
        code, output = self.run_requirement()
        self.assertEqual(code, 3)
        self.assertNotIn("sudus: LCT-001: pass", output)

    def test_terminating_the_cli_reaps_its_owned_collector(self):
        package = Path(__file__).parent
        for name in ("__init__.py", "acceptance.py", "cases.py", "identity.py", "run.py"):
            shutil.copyfile(package / name, self.root / "live_contract_checks" / name)
        self.collect("import os, time\nfrom pathlib import Path\nPath('collector.pid').write_text(str(os.getpid()))\ntime.sleep(30)\n")
        process = subprocess.Popen([sys.executable, "-B", "-m", "live_contract_checks.run", "LCT-001"],
                                   cwd=self.root, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        collector_pid = None
        try:
            marker = self.root / "collector.pid"
            deadline = time.monotonic() + 5
            while not marker.exists() and time.monotonic() < deadline and process.poll() is None:
                time.sleep(0.02)
            self.assertTrue(marker.exists(), "collector never started")
            collector_pid = int(marker.read_text())
            process.terminate()
            stdout, stderr = process.communicate(timeout=5)
            with self.assertRaises(ProcessLookupError):
                os.kill(collector_pid, 0)
            self.assertNotIn(b"sudus: LCT-001: pass", stdout)
            self.assertEqual(stderr, b"")
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=5)
            if collector_pid is not None:
                try:
                    os.killpg(collector_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    unittest.main()
