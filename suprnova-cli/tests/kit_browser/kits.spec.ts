import { expect, test, type Page, type Request, type Route } from '@playwright/test';

const kit = process.env.KIT_NAME!;
const password = 'browser-kit-password-42';
const appName = 'Kit Browser App';
const navigation = new WeakMap<Page, { loads: number; violations: string[] }>();

function partial(prop: string): (request: Request) => boolean {
  return request => new URL(request.url()).pathname === '/dashboard'
    && request.headers()['x-inertia-partial-data'] === prop;
}

/** Hold a real response until the assertion explicitly releases it. */
async function hold(page: Page, pattern: string, matches: (request: Request) => boolean) {
  let received!: (route: Route) => void;
  const requested = new Promise<Route>(resolve => { received = resolve; });
  const handler = async (route: Route) => {
    if (matches(route.request())) received(route);
    else await route.continue();
  };
  await page.route(pattern, handler);
  return {
    requested,
    async release(route: Route) {
      // Fetch upstream only now: no fabricated page props or success status.
      const response = await route.fetch();
      await route.fulfill({ response });
      await page.unroute(pattern, handler);
      return response;
    },
  };
}

async function register(page: Page, name: string, email: string) {
  await page.goto('/register');
  await expect(page.getByRole('button', { name: 'Register', exact: true })).toBeVisible();
  // K06/PAR-077: all subsequent app visits must remain in this document.
  const observed = { loads: 0, violations: [] as string[] };
  navigation.set(page, observed);
  page.on('load', () => { observed.loads++; });
  page.on('request', request => {
    if (request.isNavigationRequest() && request.frame() === page.mainFrame()) {
      observed.violations.push(`document visit: ${request.method()} ${request.url()}`);
    }
    const path = new URL(request.url()).pathname;
    if (['/register', '/login', '/logout', '/notes'].includes(path)
      && request.method() === 'POST' && request.headers()['x-inertia'] !== 'true') {
      observed.violations.push(`missing X-Inertia: ${request.method()} ${path}`);
    }
  });
  await page.locator('#name').fill(name);
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(password);
  await page.locator('#password_confirmation').fill(password);
  const processing = await hold(page, '**/register', request => request.method() === 'POST');
  const answer = page.waitForResponse(response => response.request().method() === 'POST'
    && new URL(response.url()).pathname === '/register');
  await page.getByRole('button', { name: 'Register', exact: true }).click();
  const route = await processing.requested;
  // K20/PAR-079: registration Form disables its button while processing.
  await expect(page.locator('form button[type="submit"]')).toBeDisabled();
  await route.continue();
  await page.unroute('**/register');
  expect([302, 303], `${kit}: registration Form response`).toContain((await answer).status());
  await expect(page).toHaveURL(/\/(verify-email|dashboard)$/);
  await expect(page.getByRole('status')).toHaveText(`Welcome, ${name}.`);
}

async function csrf(page: Page) {
  const cookie = (await page.context().cookies()).find(cookie => cookie.name === 'XSRF-TOKEN');
  expect(cookie, `${kit}: CSRF cookie`).toBeDefined();
  return decodeURIComponent(cookie!.value);
}

async function seedNotes(page: Page, count = 25) {
  // PAR-080: seed through the authenticated application endpoint, never SQL.
  for (let index = 0; index < count; index++) {
    const response = await page.request.post('/notes', {
      headers: { 'X-XSRF-TOKEN': await csrf(page), 'X-Inertia': 'true' },
      data: { title: `Kit note ${String(index).padStart(2, '0')}`, body: `Private body ${index}` },
      maxRedirects: 0,
    });
    expect([302, 303], `${kit}: POST /notes ${index}: ${await response.text()}`).toContain(response.status());
  }
}

async function dashboard(page: Page) {
  // VerifyEmail is a guest-layout page with an application link too.
  const response = page.waitForResponse(response => new URL(response.url()).pathname === '/dashboard'
    && !response.request().headers()['x-inertia-partial-data']);
  await page.getByRole('link', { name: /dashboard/i }).first().click();
  expect((await response).request().headers()['x-inertia']).toBe('true');
  await expect(page.getByRole('heading', { name: 'Dashboard', exact: true })).toBeVisible();
}

async function visit(page: Page, name: 'Notes' | 'Dashboard') {
  const path = name === 'Notes' ? '/notes' : '/dashboard';
  const response = page.waitForResponse(response => new URL(response.url()).pathname === path
    && !response.request().headers()['x-inertia-partial-data']);
  await page.locator('nav').getByRole('link', { name, exact: true }).click();
  expect((await response).request().headers()['x-inertia']).toBe('true');
  await expect(page).toHaveURL(new RegExp(`${path}$`));
  await expect(page).toHaveTitle(`${name} - ${appName}`);
}

function rows(page: Page) { return page.locator('main a[href*="/notes/"]'); }

async function filledNotes(page: Page) {
  await seedNotes(page);
  await dashboard(page);
  await visit(page, 'Notes');
  await expect(rows(page).first()).toBeVisible();
}

test.beforeEach(async ({ page }, info) => {
  page.on('pageerror', error => console.error(`${kit}: browser error: ${error.message}`));
  page.on('requestfailed', request => console.error(
    `${kit}: request failed: ${request.method()} ${request.url()}: ${request.failure()?.errorText}`,
  ));
  await register(page, 'Kit Reader', `${kit}-${info.testId}@example.test`);
});

test.afterEach(async ({ page }) => {
  const observed = navigation.get(page);
  // A failure before the initial page mounts has no navigation baseline.
  if (!observed) return;
  // K06/PAR-077: every exercised in-app navigation is a client visit.
  expect.soft(observed.loads, `${kit}: full document loads after Register`).toBe(0);
  expect.soft(observed.violations, `${kit}: navigation headers`).toEqual([]);
});

test('PAR-076/077/078: title, one-shot registration toast, persistent layout and heading', async ({ page }) => {
  // K17/PAR-078: registration's welcome toast is gone on the next visit.
  await dashboard(page);
  await expect(page.getByRole('status')).toHaveCount(0);
  // K07/PAR-076/077: Head appends the application name.
  await expect(page).toHaveTitle(`Dashboard - ${appName}`);
  // K16/PAR-077: a client-only state marker attached to the layout's nav
  // survives visits. No shipped layout or generated kit is modified.
  await page.locator('nav').evaluate(nav => {
    const marker = document.createElement('input');
    marker.id = 'kit-layout-state';
    marker.setAttribute('aria-label', 'Layout state');
    nav.append(marker);
  });
  await page.locator('#kit-layout-state').fill('kept across visits');
  await visit(page, 'Notes');
  await expect(page.locator('#kit-layout-state')).toHaveValue('kept across visits');
  await expect(page.getByRole('heading', { name: 'Dashboard', exact: true })).toHaveCount(0);
  await visit(page, 'Dashboard');
  await expect(page.locator('#kit-layout-state')).toHaveValue('kept across visits');
  await expect(page.getByRole('heading', { name: 'Dashboard', exact: true })).toBeVisible();
});

test('PAR-079/080: Deferred stats and stats-only polling', async ({ page }) => {
  await seedNotes(page);
  const deferred = await hold(page, '**/dashboard', partial('stats'));
  await dashboard(page);
  const request = await deferred.requested;
  // K12/PAR-079: fallback is observable before the deferred reply arrives.
  await expect(page.getByText('Counting your notes...', { exact: true })).toBeVisible();
  expect(request.request().headers()['x-inertia']).toBe('true');
  const answer = await deferred.release(request);
  expect((await answer.json()).props.stats).toMatchObject({ notes: 25, written_today: 25 });
  await expect(page.getByText('Counting your notes...', { exact: true })).toHaveCount(0);
  // K12/PAR-079: both numbers render after the real reply.
  await expect(page.locator('main').getByText('25', { exact: true })).toHaveCount(2);
  // K11/PAR-079: the next stats-only request is the first poll, after the
  // deferred request finished. No timer rewrite or arbitrary sleep.
  const poll = await page.waitForRequest(partial('stats'), { timeout: 20_000 });
  expect(poll.headers()['x-inertia-partial-component']).toBe('Dashboard');
  expect(poll.headers()['x-inertia']).toBe('true');
});

test('PAR-079/080: WhenVisible requests recent notes only after scrolling', async ({ page }) => {
  await seedNotes(page);
  const recent: Request[] = [];
  page.on('request', request => { if (partial('recent_notes')(request)) recent.push(request); });
  const stats = page.waitForResponse(response => partial('stats')(response.request()));
  await dashboard(page);
  await stats;
  await expect(page.locator('main').getByText('25', { exact: true })).toHaveCount(2);
  expect(recent, `${kit}: recent_notes requested before scrolling`).toHaveLength(0);
  // K13/PAR-079: optional prop requested only at the visibility boundary.
  const visible = page.waitForResponse(response => partial('recent_notes')(response.request()));
  await page.getByText('Loading your recent notes...', { exact: true }).scrollIntoViewIfNeeded();
  const notes = await visible;
  expect((await notes.json()).props.recent_notes).toHaveLength(5);
  await expect(page.getByRole('link', { name: 'Kit note 24', exact: true })).toBeVisible();
  expect(recent).toHaveLength(1);
});

test('PAR-079/080: useHttp shows an optimistic name and rolls back a real 422', async ({ page }) => {
  await dashboard(page);
  const name = page.locator('#name');
  const form = page.locator('form').filter({ has: name });
  let delayed = await hold(page, '**/profile/name', request => request.method() === 'POST');
  await name.fill('Changed Reader');
  await form.getByRole('button').click();
  let route = await delayed.requested;
  // K15/PAR-079: the new name shows while the answer is still held.
  await expect(page.getByRole('heading', { name: 'Welcome, Changed Reader!', exact: true })).toBeVisible();
  await expect(form.getByRole('button')).toBeDisabled();
  const auth = page.waitForResponse(response => response.request().headers()['x-inertia-partial-data'] === 'auth');
  expect((await delayed.release(route)).status()).toBe(200);
  await auth;
  await expect(form.getByRole('button')).toBeEnabled();

  delayed = await hold(page, '**/profile/name', request => request.method() === 'POST');
  // Exercise the server's empty-name rule even when native required would
  // block it. This changes only browser constraint validation, not kit state.
  await form.evaluate(form => { (form as HTMLFormElement).noValidate = true; });
  await name.fill('');
  await form.getByRole('button').click();
  route = await delayed.requested;
  await expect(page.getByRole('heading', { name: 'Welcome, !', exact: true })).toBeVisible();
  const refused = await delayed.release(route);
  // PAR-080: useHttp's real JSON validation contract.
  expect(refused.status()).toBe(422);
  const error = (await refused.json()).errors.name;
  expect(error).toBeTruthy();
  // K15/PAR-079: rollback plus the actual field error on screen.
  await expect(page.getByRole('heading', { name: 'Welcome, Changed Reader!', exact: true })).toBeVisible();
  await expect(form.getByText(Array.isArray(error) ? error[0] : error, { exact: true })).toBeVisible();
});

test('PAR-079/080: InfiniteScroll appends a second cursor page', async ({ page }) => {
  await seedNotes(page);
  await dashboard(page);
  const cursor = await hold(page, '**/notes?**', request => new URL(request.url()).searchParams.has('cursor'));
  await visit(page, 'Notes');
  await expect(rows(page)).toHaveCount(10);
  // K14/PAR-079/080: scroll, then observe cursor metadata and merged rows.
  await rows(page).last().scrollIntoViewIfNeeded();
  const route = await cursor.requested;
  const url = new URL(route.request().url());
  expect(url.searchParams.get('cursor')).toBeTruthy();
  expect(url.searchParams.has('page')).toBe(false);
  expect(route.request().headers()['x-inertia']).toBe('true');
  const reply = await cursor.release(route);
  expect((await reply.json()).scrollProps.notes).toBeDefined();
  await expect.poll(() => rows(page).count()).toBeGreaterThanOrEqual(20);
  expect(await rows(page).allTextContents()).toEqual(expect.arrayContaining([
    expect.stringContaining('Kit note 00'), expect.stringContaining('Kit note 19'),
  ]));
});

test('PAR-077/079: row hover prefetches with Purpose', async ({ page }) => {
  await filledNotes(page);
  // K06/PAR-077: a real prefetch request follows hovering the row.
  const link = rows(page).first();
  const href = new URL((await link.getAttribute('href'))!, page.url()).pathname;
  const prefetch = page.waitForRequest(request => new URL(request.url()).pathname === href
    && request.headers().purpose === 'prefetch');
  await link.hover();
  expect((await prefetch).headers()['x-inertia']).toBe('true');
});

test('PAR-079: instant Notes/Show renders row props and useRemember restores search', async ({ page }) => {
  await filledNotes(page);
  const search = page.locator('#search');
  const filtered = page.waitForResponse(response => {
    const url = new URL(response.url());
    return url.pathname === '/notes' && url.searchParams.get('search') === 'Kit note';
  });
  await search.fill('Kit note');
  await search.press('Tab');
  await filtered;
  await expect(search).toHaveValue('Kit note');
  const link = rows(page).first();
  const href = (await link.getAttribute('href'))!;
  const title = (await link.locator('span').innerText()).trim();
  const pathname = new URL(href, page.url()).pathname;
  const delayed = await hold(page, `**${pathname}`, request => request.method() === 'GET');
  // Dispatch the click without hovering: a warmed prefetch would hide an
  // instant visit that incorrectly waits for its response.
  await link.dispatchEvent('click', { button: 0 });
  const route = await delayed.requested;
  // K21/PAR-079: page component and row title before server fulfilment.
  await expect(page.locator('article').getByRole('heading', { name: title, exact: true })).toBeVisible();
  await expect(page.locator('article')).not.toContainText('Private body');
  const response = await delayed.release(route);
  const props = await response.json();
  expect(props.component).toBe('Notes/Show');
  await expect(page.locator('article')).toContainText(props.props.note.body);
  // K14/PAR-079: browser history restores the remembered search filter.
  await page.goBack();
  await expect(search).toHaveValue('Kit note');

});

test('PAR-079/080: a second account sees no private notes and show answers 404', async ({ page, browser }) => {
  await filledNotes(page);
  const href = (await rows(page).first().getAttribute('href'))!;
  const context = await browser.newContext({ baseURL: process.env.KIT_BASE_URL });
  try {
    const other = await context.newPage();
    await register(other, 'Other Reader', `${kit}-other-account@example.test`);
    await dashboard(other);
    await visit(other, 'Notes');
    // PAR-079's last sentence: no other user's rows or body.
    await expect(rows(other)).toHaveCount(0);
    const response = await other.request.get(href);
    // PAR-080: ownership enforced by the real endpoint.
    expect(response.status()).toBe(404);
    expect(await response.text()).not.toContain('Private body');
    const observed = navigation.get(other)!;
    expect(observed.loads).toBe(0);
    expect(observed.violations).toEqual([]);
  } finally {
    await context.close();
  }
});

test('PAR-079/080: notes Form creates a note with processing state and toast', async ({ page }) => {
  await dashboard(page);
  await visit(page, 'Notes');
  const form = page.locator('form').filter({ has: page.locator('#title') });
  const delayed = await hold(page, '**/notes', request => request.method() === 'POST');
  await page.locator('#title').fill('Created with Form');
  await page.locator('#body').fill('A real browser submission');
  await form.getByRole('button').click();
  const route = await delayed.requested;
  // K20/PAR-079: Form uses an Inertia submission and disables while busy.
  expect(route.request().headers()['x-inertia']).toBe('true');
  await expect(form.getByRole('button')).toBeDisabled();
  await delayed.release(route);
  await expect(rows(page).filter({ hasText: 'Created with Form' })).toHaveCount(1);
  await expect(page.getByRole('status')).toHaveText('Note saved.');
});

test('PAR-077/078: sign-out Link posts and lands on Home with a toast', async ({ page }) => {
  await dashboard(page);
  // K06/PAR-077 and K17/PAR-078: layout's POST Link and flash.
  const answer = page.waitForResponse(response => new URL(response.url()).pathname === '/logout');
  await page.locator('nav').getByRole('button', { name: /sign out/i }).click();
  const response = await answer;
  expect(response.request().method()).toBe('POST');
  expect(response.request().headers()['x-inertia']).toBe('true');
  await expect(page.getByRole('status')).toHaveText('Signed out.');
  await expect(page).toHaveURL(new URL('/', process.env.KIT_BASE_URL!).href);
  await expect(page.getByRole('heading', { name: /Welcome to/ })).toBeVisible();
  await expect(page.locator('nav').getByRole('link', { name: /sign in/i })).toBeVisible();
  await expect(page.getByRole('button', { name: /sign out/i })).toHaveCount(0);
});

test('PAR-079/080: failed sign-in Form shows errors without reloading; endpoints require auth', async ({ page }) => {
  await dashboard(page);
  await page.locator('nav').getByRole('button', { name: /sign out/i }).click();
  await expect(page.getByRole('status')).toHaveText('Signed out.');
  if (!new URL(page.url()).pathname.endsWith('/login')) {
    await page.getByRole('link', { name: /sign in/i }).first().click();
  }
  await expect(page.locator('#email')).toBeVisible();
  const delayed = await hold(page, '**/login', request => request.method() === 'POST');
  await page.locator('#email').fill('nobody@example.test');
  await page.locator('#password').fill('wrong-password');
  await page.getByRole('button', { name: /sign in/i }).click();
  const route = await delayed.requested;
  // K20/PAR-079: Form stays disabled until the real validation response.
  await expect(page.getByRole('button', { name: /signing in|sign in/i })).toBeDisabled();
  await delayed.release(route);
  await expect(page.getByText('These credentials do not match our records.', { exact: true })).toBeVisible();
  await expect(page).toHaveURL(/\/login$/);
  // PAR-080: signed-out notes and name requests go to sign-in.
  for (const path of ['/notes', '/profile/name']) {
    const response = path === '/notes'
      ? await page.request.get(path, { maxRedirects: 0 })
      : await page.request.post(path, {
        headers: { 'X-XSRF-TOKEN': await csrf(page) }, data: { name: 'Denied' }, maxRedirects: 0,
      });
    expect([302, 303]).toContain(response.status());
    expect(response.headers().location).toMatch(/\/login$/);
  }
});
