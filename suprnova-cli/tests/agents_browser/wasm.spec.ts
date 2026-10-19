import { expect, test, type Page } from '@playwright/test';

// AGENT-016: a Rust front end compiled to WebAssembly, built with
// `suprnova-webmcp` (`crates/suprnova-webmcp/examples/browser_fixture.rs`),
// registers a typed page tool and the page's server tools with WebMCP. The
// tools are observed the way a built-in agent observes them, through the
// DevTools `WebMCP` domain and `document.modelContext`, and each call is
// made by `WebMCP.invokeTool`. An init script records what the page passed
// to `registerTool`, since Chromium reports only some of it back.

interface ToolResponse {
  readonly status: string;
  readonly output?: {
    readonly structuredContent?: Record<string, unknown>;
    readonly isError?: boolean;
    readonly content?: { text: string }[];
  };
}

interface Descriptor {
  readonly name: string;
  readonly title?: string;
  readonly description: string;
  readonly inputSchema: Record<string, unknown>;
  readonly annotations: Record<string, boolean>;
}

interface Recorded {
  readonly method: string;
  readonly path: string;
  readonly tool: string | null;
  readonly csrfHeader: string | null;
  readonly sessionCookie: boolean;
  readonly accept: string | null;
  readonly contentType: string | null;
  readonly status: number;
}

async function webmcp(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebMCP.enable');
  const added: string[] = [];
  const removed: string[] = [];
  const responses = new Map<string, ToolResponse>();
  let frameId = '';
  cdp.on('WebMCP.toolsAdded', (event: { tools: { name: string; frameId: string }[] }) => {
    for (const tool of event.tools) {
      added.push(tool.name);
      frameId = tool.frameId;
    }
  });
  cdp.on('WebMCP.toolsRemoved', (event: { tools: { name: string }[] }) => {
    for (const tool of event.tools) removed.push(tool.name);
  });
  cdp.on('WebMCP.toolResponded', (event: ToolResponse & { invocationId: string }) => {
    responses.set(event.invocationId, event);
  });
  return {
    added,
    removed,
    async invoke(toolName: string, input: Record<string, unknown>): Promise<ToolResponse> {
      const { invocationId } = (await cdp.send('WebMCP.invokeTool' as never, {
        frameId,
        toolName,
        input,
      } as never)) as { invocationId: string };
      await expect.poll(() => responses.has(invocationId)).toBe(true);
      return responses.get(invocationId) as ToolResponse;
    },
  };
}

/// Record every descriptor the page hands `registerTool`, before Chromium
/// keeps the parts it reports.
async function recordRegistrations(page: Page) {
  await page.addInitScript(() => {
    const recorded: unknown[] = [];
    Reflect.set(window, '__registered', recorded);
    const context = Reflect.get(document, 'modelContext') as
      | { registerTool(tool: Record<string, unknown>, options?: unknown): unknown }
      | undefined;
    if (context === undefined) return;
    const original = context.registerTool.bind(context);
    context.registerTool = (tool, options) => {
      recorded.push(
        JSON.parse(
          JSON.stringify({
            name: tool.name,
            title: tool.title,
            description: tool.description,
            inputSchema: tool.inputSchema,
            annotations: tool.annotations,
          }),
        ),
      );
      return original(tool, options);
    };
  });
}

async function descriptor(page: Page, name: string): Promise<Descriptor | undefined> {
  return page.evaluate(
    (wanted) =>
      ((Reflect.get(window, '__registered') ?? []) as { name: string }[]).find(
        (tool) => tool.name === wanted,
      ),
    name,
  ) as Promise<Descriptor | undefined>;
}

async function registered(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const context = (document as unknown as { modelContext: { getTools(): Promise<{ name: string }[]> } })
      .modelContext;
    return (await context.getTools()).map((tool) => tool.name).sort();
  });
}

/// Open the page the WebAssembly front end boots in, once it has booted.
async function openApp(page: Page) {
  await recordRegistrations(page);
  const agent = await webmcp(page);
  await page.goto('/app');
  await expect(page.locator('html')).toHaveAttribute('data-fixture', 'ready');
  return agent;
}

async function toolRequests(page: Page): Promise<Recorded[]> {
  const body = (await (await page.request.get('/test/requests')).json()) as { requests: Recorded[] };
  return body.requests;
}

test('agent_016_wasm_registers_a_typed_tool', async ({ page }) => {
  const agent = await openApp(page);
  await expect.poll(() => registered(page)).toContain('todos.add');
  await expect.poll(() => agent.added).toContain('todos.add');
  await expect(page.locator('html')).toHaveAttribute('data-registered', 'ok');

  // The page registered the schema the crate derives from the input type,
  // which is the schema the server derives for the same type.
  const passed = await descriptor(page, 'todos.add');
  const derived = JSON.parse((await page.locator('html').getAttribute('data-expected-schema')) ?? 'null');
  const server = await (await page.request.get('/test/schema')).json();
  expect(passed?.inputSchema).toEqual(derived);
  expect(derived).toEqual(server);
  expect(passed?.title).toBe('Add todo');
  expect(passed?.description).toBe('Add an item to the visible todo list.');
  expect(passed?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
    debugging: false,
  });

  // A call reaches the typed handler; input that breaks a rule comes back
  // as an error result naming the field as the server would.
  const added = await agent.invoke('todos.add', { todoTitle: 'Buy milk' });
  expect(added.status).toBe('Completed');
  expect(added.output?.isError).toBe(false);
  expect(added.output?.structuredContent).toEqual({ added: 'Buy milk', count: 1 });
  await expect(page.locator('html')).toHaveAttribute('data-todos', '1');
  const refused = await agent.invoke('todos.add', { todoTitle: '' });
  expect(refused.output?.isError).toBe(true);
  expect(refused.output?.content?.[0]?.text).toBe(
    'The input is not valid:\n- todoTitle: The todo title field must be between 1 and 80 characters.',
  );
  await expect(page.locator('html')).toHaveAttribute('data-todos', '1');
});

test('agent_016_dropping_the_handle_unregisters_the_tool', async ({ page }) => {
  const agent = await openApp(page);
  await expect.poll(() => registered(page)).toContain('todos.add');
  await page.evaluate(() => (Reflect.get(window, 'fixture') as { dropTodoTool(): void }).dropTodoTool());
  await expect.poll(() => agent.removed).toContain('todos.add');
  await expect.poll(() => registered(page)).not.toContain('todos.add');
});

test('agent_016_wasm_server_tool_call_carries_session_csrf_and_header', async ({ page }) => {
  await page.goto('/login/alice');
  const agent = await openApp(page);
  // The server lists `todos.save` for a signed-in viewer, and the front end
  // registered it from the page's element.
  await expect.poll(() => registered(page)).toEqual(['todos.add', 'todos.save']);
  const passed = await descriptor(page, 'todos.save');
  expect(passed?.title).toBe('Save todo');
  expect(passed?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
    debugging: false,
  });
  await page.request.get('/test/reset');

  const saved = await agent.invoke('todos.save', { title: 'Ship it' });
  expect(saved.status).toBe('Completed');
  expect(saved.output?.isError).toBe(false);
  // The route saw the signed-in user and the session the page belongs to.
  expect(saved.output?.structuredContent).toEqual({ saved: 'Ship it', user: 'alice', session: 'alice' });

  const [call, ...rest] = await toolRequests(page);
  expect(rest).toEqual([]);
  expect(call).toEqual({
    method: 'POST',
    path: '/todos',
    tool: 'todos.save',
    csrfHeader: 'X-CSRF-TOKEN',
    sessionCookie: true,
    accept: 'application/json',
    contentType: 'application/json',
    status: 200,
  });
  // A tool not marked read-only refreshes the front end's data once the
  // server answered.
  await expect(page.locator('html')).toHaveAttribute('data-refreshes', '1');
});
