#!/usr/bin/env python3
"""Tests for inertia_js.cjs, the Inertia.js surface extractor.

Run: python3 feature-map/tools/laravel/test_inertia_js.py

The extractor parses with TypeScript's own parser, which generate.sh installs
at target/feature-map/laravel/typescript-<version>; TEST_INERTIA_JS_TYPESCRIPT
names another install (the directory holding typescript's package.json).
Each test writes a small Inertia-shaped monorepo (packages/<name>/package.json
with an exports map, sources under src/) and runs the extractor over it.
"""
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOOL = HERE / "inertia_js.cjs"
REPO = HERE.parents[2]


def typescript_dir():
    named = os.environ.get("TEST_INERTIA_JS_TYPESCRIPT")
    if named:
        return Path(named)
    installs = sorted((REPO / "target" / "feature-map" / "laravel").glob("typescript-*/node_modules/typescript"))
    if not installs:
        raise SystemExit("no TypeScript install: run feature-map/tools/laravel/generate.sh once, "
                         "or set TEST_INERTIA_JS_TYPESCRIPT to a typescript package directory")
    return installs[-1]


TYPESCRIPT = typescript_dir()


def package(root, name, exports, files, private=False):
    pkg = root / "packages" / name
    (pkg / "src").mkdir(parents=True)
    manifest = {"name": f"@inertiajs/{name}", "version": "3.8.0", "exports": exports}
    if private:
        manifest["private"] = True
    (pkg / "package.json").write_text(json.dumps(manifest))
    for rel, text in files.items():
        path = pkg / "src" / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def run(root):
    out = root / "out.jsonl"
    proc = subprocess.run(["node", str(TOOL), str(TYPESCRIPT), str(root), str(out)],
                          capture_output=True, text=True)
    records = {}
    if proc.returncode == 0:
        for line in out.read_text().splitlines():
            r = json.loads(line)
            records[r["id"]] = r
    return proc, records


CORE_EXPORTS = {".": {"import": "./dist/index.js"}, "./server": {"import": "./dist/server.js"}}

CORE = {
    "index.ts": (
        "import { Router } from './router'\n"
        "export { objectToFormData } from './formData'\n"
        "export * from './types'\n"
        "export { type Router }\n"
        "export const router = new Router()\n"
    ),
    "router.ts": "export class Router {\n  visit(url: string): void {}\n  private secret(): void {}\n}\n",
    "formData.ts": (
        "export function objectToFormData(source: object, form: FormData = new FormData()): FormData {\n"
        "  return form\n"
        "}\n"
    ),
    "types.ts": "export type Config = { nonce?: string }\nexport interface Page { url: string }\n",
    "server.ts": "export default (render: unknown): unknown => render\n",
    "request.ts": (
        "const MERGE = 'X-Inertia-Infinite-Scroll-Merge-Intent'\n"
        "export function headers(version: string) {\n"
        "  const h: Record<string, unknown> = { 'X-Inertia': true, [MERGE]: 'append' }\n"
        "  h['X-Inertia-Version'] = version\n"
        "  return h\n"
        "}\n"
    ),
    "response.ts": (
        "export class Response {\n"
        "  getHeader(name: string): string { return name }\n"
        "  stale(): boolean { return this.getHeader('x-inertia-version') !== '' }\n"
        "  redirect(): string { return this.getHeader('x-inertia-redirect') }\n"
        "}\n"
    ),
}


class ExtractorTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_records_every_export_at_its_declaration_across_packages(self):
        package(self.root, "core", CORE_EXPORTS, CORE)
        package(self.root, "svelte", {".": {"svelte": "./dist/index.js"}}, {
            "index.ts": (
                "export { router } from '@inertiajs/core'\n"
                "export { default as Link } from './Link.svelte'\n"
                "export { default as createInertiaApp } from './createInertiaApp'\n"
            ),
            "Link.svelte": "<script lang=\"ts\">\n  let { href }: { href: string } = $props()\n</script>\n<a {href}>x</a>\n",
            "createInertiaApp.ts": (
                "export default function createInertiaApp(options: { id: string }): void\n"
                "export default function createInertiaApp(options?: { id?: string }): void {}\n"
            ),
        })
        package(self.root, "internal", {".": {"import": "./dist/index.js"}}, {"index.ts": "export const x = 1\n"},
                private=True)
        proc, records = run(self.root)
        self.assertEqual(proc.returncode, 0, proc.stderr)

        fn = records["@inertiajs/core objectToFormData"]
        self.assertEqual((fn["kind"], fn["file"], fn["line"]), ("ts function", "packages/core/src/formData.ts", 1))
        self.assertEqual(fn["package"], "inertiajs/inertia")
        self.assertIn("form: FormData = new FormData()", fn["details"]["signature"])
        self.assertNotIn("return form", fn["details"]["signature"])

        # export * brings every name of the module; a type re-export resolves to its declaration.
        self.assertEqual(records["@inertiajs/core Page"]["kind"], "ts interface")
        self.assertEqual(records["@inertiajs/core Config"]["file"], "packages/core/src/types.ts")
        self.assertEqual(records["@inertiajs/core router"]["kind"], "ts const")
        # A local re-export of an imported name resolves through the import.
        self.assertEqual(records["@inertiajs/core Router"]["file"], "packages/core/src/router.ts")
        self.assertEqual(records["@inertiajs/core/server default"]["kind"], "ts function")

        # A re-export from another package cites the declaring package.
        router = records["@inertiajs/svelte router"]
        self.assertEqual(router["file"], "packages/core/src/index.ts")
        self.assertEqual(router["details"]["declared_in"], "@inertiajs/core")

        # A Svelte component is cited at its file; an overloaded default export is one export.
        link = records["@inertiajs/svelte Link"]
        self.assertEqual((link["kind"], link["file"], link["line"]), ("svelte component", "packages/svelte/src/Link.svelte", 1))
        self.assertEqual(records["@inertiajs/svelte createInertiaApp"]["line"], 1)

        # A private package publishes nothing.
        self.assertFalse(any(i.startswith("@inertiajs/internal") for i in records))

    def test_records_the_headers_the_client_sends_and_reads(self):
        package(self.root, "core", CORE_EXPORTS, CORE)
        proc, records = run(self.root)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        version = records["header X-Inertia-Version"]
        self.assertEqual(version["kind"], "protocol header")
        self.assertEqual(version["details"], {"sent_at": ["packages/core/src/request.ts:4"],
                                              "read_at": ["packages/core/src/response.ts:3"]})
        self.assertEqual(records["header X-Inertia"]["details"]["sent_at"], ["packages/core/src/request.ts:3"])
        # A header named through a constant is sent where the constant is used.
        self.assertEqual(records["header X-Inertia-Infinite-Scroll-Merge-Intent"]["details"]["sent_at"],
                         ["packages/core/src/request.ts:3"])
        self.assertEqual(records["header X-Inertia-Redirect"]["details"],
                         {"sent_at": [], "read_at": ["packages/core/src/response.ts:4"]})

    def test_a_line_move_changes_no_signature_and_a_parameter_does(self):
        package(self.root, "core", CORE_EXPORTS, CORE)
        _, before = run(self.root)
        form = self.root / "packages/core/src/formData.ts"
        form.write_text("\n\n" + form.read_text())
        _, moved = run(self.root)
        key = "@inertiajs/core objectToFormData"
        self.assertEqual(moved[key]["line"], 3)
        self.assertEqual(moved[key]["sig_hash"], before[key]["sig_hash"])
        self.assertEqual(moved["header X-Inertia-Version"]["sig_hash"], before["header X-Inertia-Version"]["sig_hash"])
        form.write_text(form.read_text().replace("form: FormData", "form: FormData | null"))
        _, changed = run(self.root)
        self.assertNotEqual(changed[key]["sig_hash"], before[key]["sig_hash"])

    def test_an_unresolvable_re_export_stops_the_run(self):
        core = dict(CORE)
        core["index.ts"] += "export { missing } from './formData'\n"
        package(self.root, "core", CORE_EXPORTS, core)
        proc, _ = run(self.root)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("packages/core/src/index.ts:6: packages/core/src/formData.ts does not export missing", proc.stderr)
        self.assertFalse((self.root / "out.jsonl").exists(), "nothing is written when the run stops")

    def test_a_header_neither_sent_nor_read_stops_the_run(self):
        core = dict(CORE)
        core["log.ts"] = "export function log(): void { console.log('X-Inertia-Version') }\n"
        package(self.root, "core", CORE_EXPORTS, core)
        proc, _ = run(self.root)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("packages/core/src/log.ts:1: X-Inertia-Version is neither sent nor read here", proc.stderr)


if __name__ == "__main__":
    unittest.main()
