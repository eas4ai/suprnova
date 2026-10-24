(() => {
  'use strict';
  const bar = document.getElementById('suprnova-debugbar');
  const initial = document.getElementById('suprnova-debugbar-record');
  if (!bar || !initial || bar.dataset.ready) return;
  bar.dataset.ready = 'true';
  const nav = bar.querySelector('[data-panels]');
  const section = bar.querySelector('[data-panel]');
  const title = section.querySelector('[data-title]');
  const text = section.querySelector('pre');
  const status = bar.querySelector('[data-readout]');
  const keep = Math.max(1, Number(bar.dataset.keep) || 100);
  const records = new Map();
  const pending = new Set();
  const selector = document.createElement('select');
  selector.setAttribute('aria-label', 'Recorded request');
  nav.append(selector);
  let current;
  let open;
  const panels = ['request', 'timings', 'memory', 'queries', 'transactions', 'cache', 'events', 'mail', 'notifications', 'queue', 'render_cache', 'live', 'error'];
  const buttons = panels.map(panel => {
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = panel.replace('_', ' ');
    button.setAttribute('aria-expanded', 'false');
    button.addEventListener('click', () => { open = open === panel ? undefined : panel; render(); });
    nav.append(button);
    return button;
  });
  function render() {
    if (!current) return;
    section.hidden = !open;
    buttons.forEach((button, index) => button.setAttribute('aria-expanded', String(panels[index] === open)));
    title.textContent = open === 'memory' ? current.memory_measurement : open || '';
    const value = open === 'memory'
      ? { peak_rss_growth_bytes: current.peak_rss_growth_bytes, measurement: current.memory_measurement }
      : current[open] === undefined ? current.panels[open] : current[open];
    text.textContent = JSON.stringify(value, null, 2) || '';
    status.textContent = `${current.status} | ${current.duration_ms.toFixed(1)} ms`;
  }
  function accept(record) {
    if (records.has(record.id)) { current = records.get(record.id); selector.value = record.id; render(); return; }
    records.set(record.id, record);
    const option = document.createElement('option');
    option.value = record.id;
    option.textContent = `${record.request.method} ${record.request.path}`;
    selector.append(option);
    // Match the server's finite retention without accumulating browser records forever.
    while (records.size > keep) { records.delete(records.keys().next().value); selector.remove(0); }
    current = record;
    selector.value = record.id;
    render();
  }
  selector.addEventListener('change', () => { current = records.get(selector.value); render(); });
  accept(JSON.parse(initial.textContent));
  const fetchOriginal = window.fetch;
  // Resolve under the application's path prefix, which is also embedded by the server.
  const endpoint = new URL(initial.dataset.endpoint || '/_debugbar/', location.origin);
  async function observe(id, responseUrl) {
    if (!id || records.has(id) || pending.has(id)) return;
    let url;
    try { url = new URL(responseUrl, location.href); } catch { return; }
    if (url.origin !== location.origin || url.pathname.startsWith(endpoint.pathname)) return;
    pending.add(id);
    try {
      const response = await fetchOriginal.call(window, new URL(encodeURIComponent(id), endpoint), { credentials: 'same-origin', headers: { Accept: 'application/json' }, cache: 'no-store' });
      if (!response.ok) throw new Error(`record unavailable (${response.status})`);
      accept(await response.json());
    } catch (error) { status.textContent = `Debug bar: ${error.message}`; }
    finally { pending.delete(id); }
  }
  if (fetchOriginal) window.fetch = async function (...args) {
    const response = await fetchOriginal.apply(this, args);
    void observe(response.headers.get('x-debugbar-id'), response.url);
    return response;
  };
  const sendOriginal = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function (...args) {
    this.addEventListener('loadend', () => { void observe(this.getResponseHeader('x-debugbar-id'), this.responseURL); }, { once: true });
    return sendOriginal.apply(this, args);
  };
})();
