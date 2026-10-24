(() => {
  const host = document.currentScript.previousElementSibling;
  const input = host.querySelector('input');
  const results = host.querySelector('ul');
  const status = host.querySelector('[role="status"]');
  let index;
  let revision = 0;
  input.addEventListener('input', async () => {
    const current = ++revision;
    const words = input.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
    results.replaceChildren();
    status.textContent = '';
    if (!words.length) return;
    try {
      index ??= fetch(host.dataset.index, {credentials: 'same-origin', cache: 'no-store'})
        .then(response => {
          if (!response.ok) throw new Error('Search unavailable');
          return response.json();
        });
      const pages = await index;
      if (current !== revision) return;
      const hits = pages.filter(page => {
        const text = [page.title, ...page.headings.map(h => h.title), page.text].join(' ').toLowerCase();
        return words.every(word => text.includes(word));
      }).slice(0, 20);
      for (const page of hits) {
        const li = document.createElement('li');
        const link = document.createElement('a');
        link.href = page.address;
        link.textContent = page.title;
        li.append(link);
        results.append(li);
      }
      status.textContent = hits.length ? `${hits.length} results` : 'No matching pages';
    } catch {
      if (current !== revision) return;
      index = undefined;
      status.textContent = 'Search is unavailable. Try again.';
    }
  });
})();
