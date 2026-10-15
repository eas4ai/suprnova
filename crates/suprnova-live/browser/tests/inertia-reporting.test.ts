import { readFile } from "node:fs/promises";
import vm from "node:vm";
import { resolve } from "node:path";
import { REPORTING_SECRET_CASES } from "./support/reporting-secrets.js";
import { build } from "esbuild";
import { describe, expect, it, vi } from "vitest";

const root = new URL("../../../../suprnova-cli/src/templates/files/frontend/", import.meta.url);

async function starter(
  kit: "react" | "vue" | "svelte",
  dsn: string | undefined,
  scenario: { failHydration?: boolean; sdkFails?: boolean } = {},
) {
  const filename = kit === "react" ? "main.tsx.tpl" : "main.ts.tpl";
  const source = await readFile(new URL(`${kit}/src/${filename}`, root), "utf8");
  const sentry = `globalThis.loads++; if (globalThis.sdkFails) throw new Error("SDK blocked"); export function init(options) { globalThis.init=options; } export function captureException(error) { globalThis.errors.push(error); } export function captureMessage(message) { globalThis.errors.push(message); } export function reactErrorHandler() { return captureException; } export function captureConsoleIntegration() { return { name: 'CaptureConsole' }; } export function vueIntegration(options) { globalThis.vue=options; return { name: 'Vue' }; } export function addIntegration() {}`;
  const framework = `export function createInertiaApp(options) { globalThis.options=options; return Promise.resolve(); } export function createRoot(el, options) { globalThis.root=options; return { render() {} }; } export function hydrateRoot(el, app, options) { globalThis.root=options; } export function h() {} export function createApp() { return { config: {}, use() {}, mount() {} }; } export const createSSRApp=createApp; export function hydrate() { if(globalThis.failHydration) throw new Error('hydration failure'); } export function mount() { globalThis.mounts++; } export function jsx() { return {}; } export const layout=undefined; export const title=undefined; export const pageTitle=undefined; export default {};`;
  const result = await build({
    stdin: { contents: source, loader: kit === "react" ? "tsx" : "ts", resolveDir: process.cwd() },
    bundle: true,
    write: false,
    format: "iife",
    jsx: "automatic",
    define: { "import.meta.env.VITE_REPORTING_DSN": JSON.stringify(dsn ?? "") },
    plugins: [
      {
        name: "starter-test",
        setup(builder) {
          builder.onResolve({ filter: /.*/ }, (args) => ({ path: args.path, namespace: "mock" }));
          builder.onLoad({ filter: /.*/, namespace: "mock" }, (args) => ({
            contents: args.path.startsWith("@sentry/")
              ? sentry
              : args.path.endsWith(".css")
                ? ""
                : framework,
            loader: "js",
          }));
        },
      },
    ],
  });
  const state = {
    ...scenario,
    loads: 0,
    mounts: 0,
    replacements: 0,
    errors: [] as unknown[],
    document: { querySelector: () => null },
    console: { ...console, error: vi.fn(), warn: vi.fn() },
    URL,
    URLSearchParams,
    setTimeout,
    clearTimeout,
  } as Record<string, unknown>;
  vm.runInNewContext(result.outputFiles[0]?.text ?? "", state);
  await new Promise((resolve) => setTimeout(resolve, 0));
  const options = state["options"] as { setup: (input: unknown) => Promise<void> | void };
  await options.setup({
    el: {
      hasAttribute: () => true,
      replaceChildren: () => {
        state["replacements"] = Number(state["replacements"]) + 1;
      },
    },
    App: () => null,
    props: {},
    plugin: {},
  });
  return state;
}

describe("REPORT-007 Inertia browser reporters", () => {
  it("reports a silently recoverable Svelte hydration failure and mounts the fallback", async () => {
    const state = await starter("svelte", "https://public@tracker.test/42", {
      failHydration: true,
    });
    expect(state["errors"]).toHaveLength(1);
    expect(state["mounts"]).toBe(1);
    expect(state["replacements"]).toBe(1);
  });

  for (const kit of ["react", "vue", "svelte"] as const) {
    it(`${kit} resolves Vite env files from the application's root`, async () => {
      const source = await readFile(new URL(`${kit}/vite.config.ts.tpl`, root), "utf8");
      const result = await build({
        stdin: { contents: source, loader: "ts", resolveDir: process.cwd() },
        bundle: true,
        write: false,
        format: "iife",
        plugins: [
          {
            name: "vite-config-test",
            setup(builder) {
              builder.onResolve({ filter: /.*/ }, (args) => ({
                path: args.path,
                namespace: "mock",
              }));
              builder.onLoad({ filter: /.*/, namespace: "mock" }, () => ({
                contents:
                  "export function defineConfig(config) { globalThis.config = config; return config; } export default function plugin() { return {}; } export const svelte=plugin;",
                loader: "js",
              }));
            },
          },
        ],
      });
      const state = { process: { env: {} } } as Record<string, unknown>;
      vm.runInNewContext(result.outputFiles[0]?.text ?? "", state);
      const configure = state["config"] as (mode: { isSsrBuild: boolean }) => { envDir?: string };
      for (const isSsrBuild of [false, true]) {
        expect(resolve("/application/frontend", configure({ isSsrBuild }).envDir ?? ".")).toBe(
          "/application",
        );
      }
    });
    it(`${kit} redacts complete credential headers, quoted secrets and encoded names`, async () => {
      const state = await starter(kit, "https://public@tracker.test/42");
      const init = state["init"] as { beforeSend: (event: Record<string, unknown>) => unknown };
      for (const { text, secrets } of REPORTING_SECRET_CASES) {
        const filtered = init.beforeSend({
          message: text,
          exception: {
            values: [
              {
                type: text,
                value: text,
                stacktrace: { frames: [{ filename: text, function: text }] },
              },
            ],
          },
        });
        for (const secret of secrets) expect(JSON.stringify(filtered)).not.toContain(secret);
      }
    });
    it(`${kit} loads no SDK without its public DSN`, async () => {
      const state = await starter(kit, undefined);
      expect(state["loads"]).toBe(0);
    });
    it(`${kit} still mounts if the SDK chunk cannot load`, async () => {
      const state = await starter(kit, "https://public@tracker.test/42", { sdkFails: true });
      expect(state["options"]).toBeDefined();
      expect(state["init"]).toBeUndefined();
    });
    it(`${kit} lazily initializes its official SDK before hydration`, async () => {
      const state = await starter(kit, "https://public@tracker.test/42");
      expect(state["loads"]).toBe(1);
      const integrationOptions = state["init"] as {
        integrations: (defaults: { name: string }[]) => { name: string }[];
      };
      expect(
        integrationOptions
          .integrations([{ name: "BrowserApiErrors" }, { name: "GlobalHandlers" }])
          .map((integration) => integration.name),
      ).toEqual(["GlobalHandlers"]);
      expect(state["init"]).toMatchObject({
        dsn: "https://public@tracker.test/42",
        sendDefaultPii: false,
      });
      const init = state["init"] as {
        beforeSend: (event: Record<string, unknown>) => Record<string, unknown> | null;
      };
      const filtered = init.beforeSend({
        request: { headers: { Cookie: "session=secret" } },
        user: { email: "secret@private.test" },
        breadcrumbs: [{ message: "private" }],
        extra: { password: "secret" },
        exception: {
          values: [
            {
              value:
                "password=hunter2 Authorization: Bearer topsecret https://app.test/?token=secret",
            },
          ],
        },
      });
      expect(filtered).not.toHaveProperty("request");
      expect(filtered).not.toHaveProperty("user");
      expect(JSON.stringify(filtered)).not.toContain("hunter2");
      expect(JSON.stringify(filtered)).not.toContain("topsecret");
      expect(JSON.stringify(filtered)).not.toContain("token=secret");
      expect(init.beforeSend({ logger: "console", message: "an ordinary log" })).toBeNull();
      if (kit === "vue" || kit === "svelte") {
        const output = state["console"] as {
          error: (...args: unknown[]) => void;
          warn: (...args: unknown[]) => void;
        };
        const log = kit === "vue" ? output.error : output.warn;
        log("an ordinary diagnostic");
        expect(state["errors"]).toHaveLength(0);
        log("Hydration completed but contains mismatches.");
        expect(state["errors"]).toHaveLength(1);
      }
      if (kit === "react") {
        const options = state["root"] as { onRecoverableError: (error: Error) => void };
        options.onRecoverableError(new Error("hydration failure"));
        expect(state["errors"]).toHaveLength(1);
      }
      if (kit === "vue") expect(state["vue"]).toMatchObject({ attachProps: false });
    });
  }
});
