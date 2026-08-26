import { expect, test, type Page } from "@playwright/test";

type SourceMutation = "foreign_document" | "foreign_island" | "reregister" | "scope";

async function installSynchronousMutationDrain(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const NativeMutationObserver = window.MutationObserver;
    const registrations: {
      readonly callback: MutationCallback;
      readonly observer: MutationObserver;
    }[] = [];
    const CapturingMutationObserver = new Proxy(NativeMutationObserver, {
      construct(target, argumentsList) {
        const callback = argumentsList[0] as MutationCallback;
        const observer = Reflect.construct(target, argumentsList) as MutationObserver;
        registrations.push({ callback, observer });
        return observer;
      },
    });
    Object.defineProperty(window, "MutationObserver", {
      configurable: true,
      value: CapturingMutationObserver,
    });
    Reflect.set(window, "__suprnovaDrainMutations", () => {
      for (const { callback, observer } of registrations) {
        const records = observer.takeRecords();
        if (records.length > 0) callback(records, observer);
      }
    });
  });
}

async function dispatchAfterSourceMutation(page: Page, mutation: SourceMutation) {
  return page.evaluate((kind) => {
    interface IslandPort {
      readonly element: Element;
      authorizeRegisteredEvents(registration: unknown): object;
      dispatchRegisteredEvent(capability: object, event: unknown): unknown;
    }
    const runtime = Reflect.get(window, Symbol.for("suprnova.live.runtime.v1")) as {
      register(driver: readonly unknown[]): string;
    };
    const ports: IslandPort[] = [];
    const driver = Object.freeze([
      Symbol.for("suprnova.live.feature-driver.v1"),
      1,
      1_099_511_758_848,
      Object.freeze({}),
      (event: number, value: unknown) => {
        if (event === 1) ports.push(value as IslandPort);
        return true;
      },
    ]);
    if (runtime.register(driver) !== "registered") throw new Error("driver_registration_failed");
    const source = ports.find(({ element }) => element.id === "first-island");
    const foreign = ports.find(({ element }) => element.id === "second-island");
    if (source === undefined || foreign === undefined) throw new Error("island_port_missing");
    const sourcePort: IslandPort = source;
    const foreignPort: IslandPort = foreign;
    const registration = Object.freeze({
      descriptorBinding: "event-authority-e2e",
      events: Object.freeze([
        Object.freeze({
          cycle: Object.freeze({ kind: "forbid_repeated_island" }),
          maximumFanout: 1,
          name: "orders.updated",
          order: "per_source_sequence",
          payloadContract: "orders.updated.v1",
          schema: "json",
          source: "stream",
          targets: Object.freeze(["document"]),
          version: 1,
        }),
      ]),
    });
    const capability = sourcePort.authorizeRegisteredEvents(registration);
    let deliveries = 0;
    document.addEventListener("suprnova:orders.updated", () => {
      deliveries += 1;
    });
    const NativeCustomEvent = window.CustomEvent;
    class MutatingCustomEvent<T = unknown> extends NativeCustomEvent<T> {
      constructor(type: string, init?: CustomEventInit<T>) {
        super(type, init);
        if (kind === "foreign_document") {
          const frame = document.createElement("iframe");
          document.body.append(frame);
          const body = frame.contentDocument?.body;
          if (body === undefined) throw new Error("foreign_document_missing");
          body.append(sourcePort.element);
        } else if (kind === "foreign_island") {
          foreignPort.element.append(sourcePort.element);
        } else if (kind === "reregister") {
          const drain: unknown = Reflect.get(window, "__suprnovaDrainMutations");
          if (typeof drain !== "function") throw new Error("mutation_drain_missing");
          const invokeDrain = drain as () => void;
          sourcePort.element.remove();
          invokeDrain();
          document.body.append(sourcePort.element);
          invokeDrain();
        } else {
          sourcePort.element.setAttribute("data-suprnova-live-slot", "changed-scope");
          sourcePort.element.setAttribute("data-suprnova-live-root", "changed-scope");
        }
      }
    }
    Object.defineProperty(window, "CustomEvent", {
      configurable: true,
      value: MutatingCustomEvent,
    });

    const disposition = sourcePort.dispatchRegisteredEvent(capability, {
      event: "orders.updated",
      payload: Object.freeze({ order: 41 }),
      schemaVersion: 1,
      target: "document",
    });
    return {
      deliveries,
      disposition:
        typeof disposition === "string"
          ? disposition
          : String(Reflect.get(disposition as object, "kind")),
    };
  }, mutation);
}

for (const mutation of ["foreign_document", "foreign_island", "reregister", "scope"] as const) {
  test(`core rejects a registered event when its factory mutates source authority: ${mutation}`, async ({
    page,
  }) => {
    if (mutation === "reregister") await installSynchronousMutationDrain(page);
    await page.goto("/scenario/multipleSchedulers");
    await expect(page.locator("#first-island")).toHaveAttribute(
      "data-suprnova-live-status",
      "connected",
    );

    const result = await dispatchAfterSourceMutation(page, mutation);

    expect(result.deliveries).toBe(0);
    expect(["rejected", "retired"]).toContain(result.disposition);
  });
}
