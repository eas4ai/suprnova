import { expect, test, type Page } from "@playwright/test";

type SourceMutation = "foreign_document" | "foreign_island" | "reregister" | "scope";
type TargetMutation = "foreign_document" | "scope";
type RelationshipTarget = "child" | "parent";

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

async function dispatchAfterNamedTargetMutation(page: Page, mutation: TargetMutation) {
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
    const target = ports.find(({ element }) => element.id === "second-island");
    if (source === undefined || target === undefined) throw new Error("island_port_missing");
    const targetPort: IslandPort = target;
    const registration = Object.freeze({
      descriptorBinding: "named-target-authority-e2e",
      events: Object.freeze([
        Object.freeze({
          cycle: Object.freeze({ kind: "forbid_repeated_island" }),
          maximumFanout: 1,
          name: "orders.updated",
          order: "per_source_sequence",
          payloadContract: "orders.updated.v1",
          schema: "json",
          source: "stream",
          targets: Object.freeze(["named_island:second-scheduler-slot"]),
          version: 1,
        }),
      ]),
    });
    const capability = source.authorizeRegisteredEvents(registration);
    let deliveries = 0;
    targetPort.element.addEventListener("suprnova:orders.updated", () => {
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
          body.append(targetPort.element);
        } else {
          targetPort.element.setAttribute("data-suprnova-live-slot", "changed-target-scope");
          targetPort.element.setAttribute("data-suprnova-live-root", "changed-target-scope");
        }
      }
    }
    Object.defineProperty(window, "CustomEvent", {
      configurable: true,
      value: MutatingCustomEvent,
    });
    const disposition = source.dispatchRegisteredEvent(capability, {
      event: "orders.updated",
      payload: Object.freeze({ order: 42 }),
      schemaVersion: 1,
      target: "named_island:second-scheduler-slot",
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

async function dispatchAfterRelationshipTargetMutation(
  page: Page,
  relationship: RelationshipTarget,
) {
  return page.evaluate((targetKind) => {
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
    const parent = ports.find(
      ({ element }) => element.getAttribute("data-suprnova-live-document-key") === "primary",
    );
    const child = ports.find(
      ({ element }) => element.getAttribute("data-suprnova-live-document-key") === "child",
    );
    if (parent === undefined || child === undefined) throw new Error("island_port_missing");
    const source = targetKind === "child" ? parent : child;
    const target = targetKind === "child" ? child : parent;
    const capability = source.authorizeRegisteredEvents(
      Object.freeze({
        descriptorBinding: `relationship-target-${targetKind}`,
        events: Object.freeze([
          Object.freeze({
            cycle: Object.freeze({ kind: "forbid_repeated_island" }),
            maximumFanout: 1,
            name: "orders.updated",
            order: "per_source_sequence",
            payloadContract: "orders.updated.v1",
            schema: "json",
            source: "stream",
            targets: Object.freeze([targetKind]),
            version: 1,
          }),
        ]),
      }),
    );
    let deliveries = 0;
    target.element.addEventListener("suprnova:orders.updated", () => {
      deliveries += 1;
    });
    const NativeCustomEvent = window.CustomEvent;
    class MutatingCustomEvent<T = unknown> extends NativeCustomEvent<T> {
      constructor(type: string, init?: CustomEventInit<T>) {
        super(type, init);
        target.element.setAttribute("data-suprnova-live-slot", "changed-relationship-scope");
        target.element.setAttribute("data-suprnova-live-root", "changed-relationship-scope");
      }
    }
    Object.defineProperty(window, "CustomEvent", {
      configurable: true,
      value: MutatingCustomEvent,
    });
    const disposition = source.dispatchRegisteredEvent(capability, {
      event: "orders.updated",
      payload: Object.freeze({ order: 44 }),
      schemaVersion: 1,
      target: targetKind,
    });
    return {
      deliveries,
      disposition:
        typeof disposition === "string"
          ? disposition
          : String(Reflect.get(disposition as object, "kind")),
    };
  }, relationship);
}

async function addSecondNamedTarget(page: Page): Promise<void> {
  await page.evaluate(() => {
    const second = document.querySelector("#second-island");
    if (!(second instanceof Element)) throw new Error("second_island_missing");
    const third = second.cloneNode(true);
    if (!(third instanceof Element)) throw new Error("third_island_clone_invalid");
    third.id = "third-island";
    third.setAttribute("data-suprnova-live-document-key", "third-scheduler");
    third.removeAttribute("data-suprnova-live-status");
    third.querySelector("#second-scheduler")?.removeAttribute("id");
    document.body.append(third);
  });
  await expect(page.locator("#third-island")).toHaveAttribute(
    "data-suprnova-live-status",
    "connected",
  );
}

async function dispatchNamedFanoutAfterFirstDelivery(
  page: Page,
  mutation: "foreign_document" | "foreign_island",
) {
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
    const first = ports.find(({ element }) => element.id === "second-island");
    const later = ports.find(({ element }) => element.id === "third-island");
    if (source === undefined || first === undefined || later === undefined) {
      throw new Error("island_port_missing");
    }
    const registration = Object.freeze({
      descriptorBinding: "named-fanout-authority-e2e",
      events: Object.freeze([
        Object.freeze({
          cycle: Object.freeze({ kind: "forbid_repeated_island" }),
          maximumFanout: 2,
          name: "orders.updated",
          order: "per_source_sequence",
          payloadContract: "orders.updated.v1",
          schema: "json",
          source: "stream",
          targets: Object.freeze(["named_island:second-scheduler-slot"]),
          version: 1,
        }),
      ]),
    });
    const capability = source.authorizeRegisteredEvents(registration);
    const deliveries: string[] = [];
    first.element.addEventListener("suprnova:orders.updated", () => {
      deliveries.push("first");
      if (kind === "foreign_document") {
        const frame = document.createElement("iframe");
        document.body.append(frame);
        const body = frame.contentDocument?.body;
        if (body === undefined) throw new Error("foreign_document_missing");
        body.append(later.element);
      } else {
        source.element.append(later.element);
      }
    });
    later.element.addEventListener("suprnova:orders.updated", () => {
      deliveries.push("later");
    });
    const disposition = source.dispatchRegisteredEvent(capability, {
      event: "orders.updated",
      payload: Object.freeze({ order: 43 }),
      schemaVersion: 1,
      target: "named_island:second-scheduler-slot",
    });
    return { deliveries, disposition };
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

for (const mutation of ["foreign_document", "scope"] as const) {
  test(`core rejects a named target whose event factory mutates target authority: ${mutation}`, async ({
    page,
  }) => {
    await page.goto("/scenario/multipleSchedulers");
    await expect(page.locator("#second-island")).toHaveAttribute(
      "data-suprnova-live-status",
      "connected",
    );

    const result = await dispatchAfterNamedTargetMutation(page, mutation);

    expect(result).toEqual({ deliveries: 0, disposition: "no_target" });
  });
}

for (const relationship of ["parent", "child"] as const) {
  test(`core rejects a ${relationship} target whose event factory mutates island metadata`, async ({
    page,
  }) => {
    await page.goto("/scenario/directiveOwnership");
    await expect(page.locator('[data-suprnova-live-document-key="child"]')).toHaveAttribute(
      "data-suprnova-live-status",
      "connected",
    );

    const result = await dispatchAfterRelationshipTargetMutation(page, relationship);

    expect(result).toEqual({ deliveries: 0, disposition: "no_target" });
  });
}

for (const mutation of ["foreign_document", "foreign_island"] as const) {
  test(`core reports partial fanout when an earlier listener mutates a later named target: ${mutation}`, async ({
    page,
  }) => {
    await page.goto("/scenario/multipleSchedulers");
    await addSecondNamedTarget(page);

    const result = await dispatchNamedFanoutAfterFirstDelivery(page, mutation);

    expect(result).toEqual({
      deliveries: ["first"],
      disposition: {
        delivered: 1,
        kind: "partially_dispatched",
        reason: "target_retired",
        skipped: 1,
      },
    });
  });
}
