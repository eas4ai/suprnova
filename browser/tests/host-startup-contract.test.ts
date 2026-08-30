import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterRuntimeAssetsValidated } from "../test-host/runtime-assets.mjs";
import { describe, expect, it } from "vitest";

const ASSET_NAME = "suprnova-live.esm.js";

async function preparedArtifacts(root: string): Promise<void> {
  const content = Buffer.from("export const ready = true;\n", "utf8");
  await writeFile(join(root, ASSET_NAME), content);
  await writeFile(
    join(root, "suprnova-live.assets.json"),
    `${JSON.stringify({
      assets: [
        {
          bytes: content.byteLength,
          file: ASSET_NAME,
          sha256: createHash("sha256").update(content).digest("hex"),
        },
      ],
      schema_version: 2,
    })}\n`,
    "utf8",
  );
}

async function artifactEvidence(root: string): Promise<readonly string[]> {
  return Promise.all(
    [ASSET_NAME, "suprnova-live.assets.json"].map(async (name) => {
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

describe("Playwright host startup artifact ownership", () => {
  it("admits concurrent read-only hosts without mutating the prepared artifact set", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-live-host-startup-"));
    try {
      await preparedArtifacts(root);
      const before = await artifactEvidence(root);
      const started: string[] = [];

      await Promise.all([
        afterRuntimeAssetsValidated(root, () => started.push("reader-a")),
        afterRuntimeAssetsValidated(root, () => started.push("reader-b")),
      ]);

      expect(started.sort()).toEqual(["reader-a", "reader-b"]);
      expect(await artifactEvidence(root)).toEqual(before);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  it("fails before the host startup callback when an artifact is missing", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-live-host-startup-missing-"));
    try {
      await preparedArtifacts(root);
      await rm(join(root, ASSET_NAME));
      let started = false;

      await expect(
        afterRuntimeAssetsValidated(root, () => {
          started = true;
        }),
      ).rejects.toThrow(`runtime_asset_unavailable:${ASSET_NAME}`);
      expect(started).toBe(false);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });
});
