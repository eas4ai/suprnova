import { randomUUID } from "node:crypto";
import { rename, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { withProductionBuildLock } from "./support/production-build.js";

interface Deferred {
  readonly promise: Promise<void>;
  resolve(): void;
}

function deferred(): Deferred {
  let resolve = (): void => undefined;
  const promise = new Promise<void>((settle) => {
    resolve = settle;
  });
  return Object.freeze({ promise, resolve });
}

describe("production build isolation", () => {
  it("admits one build task at a time without losing either result", async () => {
    const isolatedLock = join(tmpdir(), `suprnova-live-production-build-test-${randomUUID()}.lock`);
    const firstEntered = deferred();
    const releaseFirst = deferred();
    const order: string[] = [];
    const first = withProductionBuildLock(async () => {
      order.push("first:start");
      firstEntered.resolve();
      await releaseFirst.promise;
      order.push("first:end");
      return "first";
    }, isolatedLock);
    await firstEntered.promise;
    const second = withProductionBuildLock(() => {
      order.push("second:start");
      order.push("second:end");
      return "second";
    }, isolatedLock);
    await Promise.resolve();

    expect(order).toEqual(["first:start"]);
    releaseFirst.resolve();
    await expect(Promise.all([first, second])).resolves.toEqual(["first", "second"]);
    expect(order).toEqual(["first:start", "first:end", "second:start", "second:end"]);
  });

  it("cannot release a replacement owner after its own lock path is retired", async () => {
    const isolatedLock = join(tmpdir(), `suprnova-live-production-build-test-${randomUUID()}.lock`);
    const retiredLock = `${isolatedLock}.retired`;
    const firstEntered = deferred();
    const releaseFirst = deferred();
    const secondEntered = deferred();
    const releaseSecond = deferred();
    const first = withProductionBuildLock(async () => {
      firstEntered.resolve();
      await releaseFirst.promise;
    }, isolatedLock);
    await firstEntered.promise;
    await rename(isolatedLock, retiredLock);
    const second = withProductionBuildLock(async () => {
      secondEntered.resolve();
      await releaseSecond.promise;
    }, isolatedLock);
    await secondEntered.promise;
    releaseFirst.resolve();
    await first;
    expect((await stat(isolatedLock)).isDirectory()).toBe(true);
    releaseSecond.resolve();
    await second;
    await rm(retiredLock, { force: true, recursive: true });
  });
});
