import { MessageChannel } from "node:worker_threads";

/** Resolves on the next event-loop task after the current microtask queue drains. */
export function eventLoopBarrier(): Promise<void> {
  return new Promise((resolve) => {
    const { port1, port2 } = new MessageChannel();
    port1.once("message", () => {
      port1.close();
      port2.close();
      resolve();
    });
    port2.postMessage(undefined);
  });
}
