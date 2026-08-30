import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterRuntimeAssetsValidated } from "../test-host/runtime-assets.mjs";
import { describe, expect, it } from "vitest";

const EXPECTED_ASSETS = [
  ["suprnova-live.classic.js", "core-classic", "core@1", "classic", "preload"],
  ["suprnova-live.esm.js", "core-esm", "core@1", "module", "modulepreload"],
  ["suprnova-live.stimulus.classic.js", "stimulus-classic", "stimulus@1", "classic", "preload"],
  ["suprnova-live.stimulus.esm.js", "stimulus-esm", "stimulus@1", "module", "modulepreload"],
  ["suprnova-live.uploads.classic.js", "uploads-classic", "uploads@1", "classic", "preload"],
  ["suprnova-live.uploads.esm.js", "uploads-esm", "uploads@1", "module", "modulepreload"],
  ["suprnova-live.async.classic.js", "async-classic", "async@1", "classic", "preload"],
  ["suprnova-live.async.esm.js", "async-esm", "async@1", "module", "modulepreload"],
] as const;

interface ManifestAsset {
  bytes: number;
  cache_control: string;
  capability: string;
  capability_version: number;
  compatible_core: string;
  content_type: string;
  file: string;
  preload_rel: string;
  role: string;
  script_kind: string;
  sha256: string;
  sri: string;
}

interface Manifest {
  assets: ManifestAsset[];
  built_at: string;
  engine_version: string;
  protocol_versions: number[];
  provenance: Record<string, unknown>;
  runtime_contract_version: number;
  schema_version: number;
  snapshot_versions: number[];
  unexpected?: boolean;
}

function assetRecord(definition: (typeof EXPECTED_ASSETS)[number], content: Buffer): ManifestAsset {
  const [file, role, capability, scriptKind, preloadRel] = definition;
  const digest = createHash("sha256").update(content).digest();
  return {
    bytes: content.byteLength,
    cache_control: "public, max-age=31536000, immutable",
    capability,
    capability_version: 1,
    compatible_core: ">=0.1.0 <0.2.0",
    content_type: "text/javascript; charset=utf-8",
    file,
    preload_rel: preloadRel,
    role,
    script_kind: scriptKind,
    sha256: digest.toString("hex"),
    sri: `sha256-${digest.toString("base64")}`,
  };
}

async function preparedArtifacts(root: string): Promise<ReadonlyMap<string, Buffer>> {
  const contents = new Map<string, Buffer>();
  const assets: ManifestAsset[] = [];
  for (const definition of EXPECTED_ASSETS) {
    const content = Buffer.from(`export const role = ${JSON.stringify(definition[1])};\n`, "utf8");
    contents.set(definition[0], content);
    assets.push(assetRecord(definition, content));
    await writeFile(join(root, definition[0]), content);
  }
  const manifest: Manifest = {
    assets,
    built_at: "1970-01-01T00:00:00.000Z",
    engine_version: "0.1.0",
    protocol_versions: [1, 2],
    provenance: {
      idiomorph: { bundled: true, license: "0BSD", name: "idiomorph", version: "0.7.4" },
    },
    runtime_contract_version: 1,
    schema_version: 2,
    snapshot_versions: [1],
  };
  await writeManifest(root, manifest);
  return contents;
}

async function readManifest(root: string): Promise<Manifest> {
  return JSON.parse(await readFile(join(root, "suprnova-live.assets.json"), "utf8")) as Manifest;
}

async function writeManifest(root: string, manifest: Manifest): Promise<void> {
  await writeFile(join(root, "suprnova-live.assets.json"), `${JSON.stringify(manifest)}\n`, "utf8");
}

async function artifactEvidence(root: string): Promise<readonly string[]> {
  return Promise.all(
    [...EXPECTED_ASSETS.map(([file]) => file), "suprnova-live.assets.json"].map(async (name) => {
      const path = join(root, name);
      const [content, metadata] = await Promise.all([readFile(path), stat(path, { bigint: true })]);
      return `${name}:${metadata.size.toString()}:${metadata.mtimeNs.toString()}:${createHash(
        "sha256",
      )
        .update(content)
        .digest("hex")}`;
    }),
  );
}

async function withArtifacts(
  name: string,
  run: (root: string, contents: ReadonlyMap<string, Buffer>) => Promise<void>,
): Promise<void> {
  const root = await mkdtemp(join(tmpdir(), `suprnova-live-${name}-`));
  try {
    const contents = await preparedArtifacts(root);
    await run(root, contents);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
}

describe("Playwright host startup artifact ownership", () => {
  it("admits concurrent read-only hosts without mutating the complete artifact set", async () => {
    await withArtifacts("host-startup", async (root) => {
      const before = await artifactEvidence(root);
      const started: string[] = [];

      await Promise.all([
        afterRuntimeAssetsValidated(root, () => started.push("reader-a")),
        afterRuntimeAssetsValidated(root, () => started.push("reader-b")),
      ]);

      expect(started.sort()).toEqual(["reader-a", "reader-b"]);
      expect(await artifactEvidence(root)).toEqual(before);
    });
  });

  it("fails before startup when one of the eight production roles is missing", async () => {
    await withArtifacts("host-startup-missing", async (root) => {
      const manifest = await readManifest(root);
      manifest.assets.pop();
      await writeManifest(root, manifest);
      let started = false;

      await expect(
        afterRuntimeAssetsValidated(root, () => {
          started = true;
        }),
      ).rejects.toThrow("runtime_asset_manifest_invalid");
      expect(started).toBe(false);
    });
  });

  it("rejects extra, duplicate, and role-to-file path mutations", async () => {
    for (const mutation of ["extra", "duplicate", "path"] as const) {
      await withArtifacts(`host-startup-${mutation}`, async (root, contents) => {
        const manifest = await readManifest(root);
        const first = manifest.assets[0];
        if (first === undefined) throw new Error("fixture_asset_missing");
        if (mutation === "extra") {
          const content = Buffer.from("export const extra = true;\n", "utf8");
          await writeFile(join(root, "extra.js"), content);
          manifest.assets.push({ ...assetRecord(EXPECTED_ASSETS[0], content), file: "extra.js" });
        } else if (mutation === "duplicate") {
          manifest.assets[manifest.assets.length - 1] = { ...first };
        } else {
          const content = contents.get(first.file);
          if (content === undefined) throw new Error("fixture_content_missing");
          first.file = "renamed.js";
          await writeFile(join(root, first.file), content);
        }
        await writeManifest(root, manifest);

        await expect(afterRuntimeAssetsValidated(root, () => undefined)).rejects.toThrow(
          "runtime_asset_manifest_invalid",
        );
      });
    }
  });

  it("rejects an otherwise valid manifest with an unknown schema field", async () => {
    await withArtifacts("host-startup-schema", async (root) => {
      const manifest = await readManifest(root);
      manifest.unexpected = true;
      await writeManifest(root, manifest);

      await expect(afterRuntimeAssetsValidated(root, () => undefined)).rejects.toThrow(
        "runtime_asset_manifest_invalid",
      );
    });
  });

  it("serves the validated byte snapshot after the source files change", async () => {
    await withArtifacts("host-startup-snapshot", async (root, contents) => {
      const validated = await afterRuntimeAssetsValidated(root, (artifacts) => artifacts);
      const [file] = EXPECTED_ASSETS[0];
      const expected = contents.get(file);
      if (expected === undefined) throw new Error("fixture_content_missing");

      await writeFile(join(root, file), "mutated after validation\n", "utf8");
      await rm(join(root, EXPECTED_ASSETS[1][0]));

      const served = validated.asset(file);
      expect(served?.bytes).toEqual(expected);
      expect(served?.contentType).toBe("text/javascript; charset=utf-8");
      expect(served?.cacheControl).toBe("public, max-age=31536000, immutable");
      served?.bytes.fill(0);
      expect(validated.asset(file)?.bytes).toEqual(expected);
      expect(validated.asset(EXPECTED_ASSETS[1][0])?.bytes).toEqual(
        contents.get(EXPECTED_ASSETS[1][0]),
      );
      expect(validated.asset("suprnova-live.assets.json")?.contentType).toBe(
        "application/json; charset=utf-8",
      );
    });
  });

  it("documents the build-owning wrapper and read-only raw host", async () => {
    const documentation = await readFile(
      new URL("../docs/exploratory-browser-qa.md", import.meta.url),
      "utf8",
    );
    expect(documentation).toContain("npm run host:static");
    expect(documentation).toContain("node test-host/server.mjs");
    expect(documentation).toMatch(/validates and serves an\s+existing completed build/u);
    expect(documentation).not.toContain(
      "The host builds the production artifacts before listening",
    );
  });
});
