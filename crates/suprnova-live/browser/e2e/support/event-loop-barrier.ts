import type { Page } from "@playwright/test";

/** Waits for one browser task boundary without relying on elapsed time. */
export async function browserTaskBarrier(page: Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        const channel = new MessageChannel();
        channel.port1.onmessage = (): void => {
          channel.port1.close();
          channel.port2.close();
          resolve();
        };
        channel.port2.postMessage(undefined);
      }),
  );
}
