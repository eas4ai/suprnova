import {expect, test} from 'bun:test';

const source = await Bun.file(new URL('../src/search.js', import.meta.url)).text();

function browser(fetcher) {
  let onInput;
  const input = {value: '', addEventListener: (_, listener) => { onInput = listener; }};
  const results = {children: [], replaceChildren() { this.children = []; }, append(child) { this.children.push(child); }};
  const status = {textContent: ''};
  const host = {dataset: {index: '/guide/v2/en/search.json'}, querySelector(selector) {
    return selector === 'input' ? input : selector === 'ul' ? results : status;
  }};
  const document = {currentScript: {previousElementSibling: host}, createElement() {
    return {children: [], append(child) { this.children.push(child); }};
  }};
  new Function('document', 'fetch', source)(document, fetcher);
  return {results, status, async search(value) { input.value = value; return onInput(); }};
}

const pages = [
  {title: 'Setup', address: '/guide/v2/en/setup', headings: [{title: 'Orbital alignment'}], text: 'Align the antenna.'},
  {title: '<img src=x onerror=alert(1)>', address: '/guide/v2/en/safe', headings: [], text: 'Inert title.'},
];

test('heading words find a page and repeated searches reuse the boot index', async () => {
  let calls = 0;
  const ui = browser(async (url, options) => {
    expect(url).toBe('/guide/v2/en/search.json');
    expect(options.cache).toBe('no-store');
    calls++;
    return {ok: true, json: async () => pages};
  });
  await ui.search('ORBITAL alignment');
  expect(ui.results.children[0].children[0].href).toBe('/guide/v2/en/setup');
  expect(ui.status.textContent).toBe('1 results');
  await ui.search('antenna');
  expect(calls).toBe(1);
  await ui.search('not found');
  expect(ui.results.children).toHaveLength(0);
  expect(ui.status.textContent).toBe('No matching pages');
  await ui.search('');
  expect(ui.status.textContent).toBe('');
});

test('titles are DOM text and search results are capped at twenty', async () => {
  const ui = browser(async () => ({ok: true, json: async () => pages}));
  await ui.search('inert');
  const link = ui.results.children[0].children[0];
  expect(link.textContent).toBe('<img src=x onerror=alert(1)>');
  expect(link.innerHTML).toBeUndefined();
  const many = browser(async () => ({ok: true, json: async () => Array.from({length: 30}, () => pages[0])}));
  await many.search('setup');
  expect(many.results.children).toHaveLength(20);
});

test('a failed index fetch is visible and the next search can retry', async () => {
  let calls = 0;
  const ui = browser(async () => ++calls === 1 ? {ok: false} : {ok: true, json: async () => pages});
  await ui.search('setup');
  expect(ui.status.textContent).toBe('Search is unavailable. Try again.');
  await ui.search('setup');
  expect(ui.results.children).toHaveLength(1);
});

test('clearing a pending search prevents stale results from appearing', async () => {
  let resolve;
  const pending = new Promise(done => { resolve = done; });
  const ui = browser(async () => ({ok: true, json: () => pending}));
  const search = ui.search('setup');
  await ui.search('');
  resolve(pages);
  await search;
  expect(ui.results.children).toHaveLength(0);
  expect(ui.status.textContent).toBe('');
});
