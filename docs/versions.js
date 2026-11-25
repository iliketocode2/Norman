/* The version switcher.
 *
 * Loaded by the current site and by every old one under v/X.Y.Z/ (see
 * scripts/docs-versions.mjs, which writes the list below and the old sites).
 * Turns the version badge into a menu of every release, each link going to
 * the same page in that release where it has one. On an old release's pages
 * it also adds a banner saying so, with the way back to the latest.
 *
 * Old sites keep the stylesheet they shipped with, so the styles live here,
 * and every colour has a fallback in case an old stylesheet lacks the token.
 */
(() => {
  const VERSIONS = [
    {"version":"0.5.0","pages":["demo.html","docs.html","index.html","playground.html"]},
    {"version":"0.4.0","pages":["demo.html","docs.html","index.html","playground.html"]},
    {"version":"0.3.0","pages":["demo.html","docs.html","index.html","playground.html"]},
    {"version":"0.2.0","pages":["index.html","playground.html","reference.html","watch.html"]},
    {"version":"0.1.0","pages":["index.html","playground.html"]},
  ];

  const script = document.currentScript;
  const LATEST = VERSIONS[0].version;
  const here = script.dataset.version || LATEST;
  const root = new URL('.', script.src);   // this file sits at the site's root
  const RELEASES = 'https://github.com/iliketocode2/Norman/releases/tag/v';

  // Pages renamed between releases: docs was reference, demo was watch.
  const ALIAS = { 'docs.html': 'reference.html', 'reference.html': 'docs.html', 'demo.html': 'watch.html', 'watch.html': 'demo.html' };

  function currentPage() {
    let last = location.pathname.split('/').pop();
    if (!last) return 'index.html';
    if (!last.includes('.')) last += '.html';   // a clean URL: /docs
    return last;
  }

  /** The same page in release v, or that release's front page. */
  function hrefFor(v) {
    const page = currentPage();
    const target = v.pages.includes(page) ? page : v.pages.includes(ALIAS[page]) ? ALIAS[page] : 'index.html';
    const dir = v.version === LATEST ? root : new URL(`v/${v.version}/`, root);
    return new URL(target === 'index.html' ? '' : target, dir).href;
  }

  const style = document.createElement('style');
  style.textContent = `
    .vs { position: relative; display: inline-block; vertical-align: middle; margin-left: 0.6rem;
      font-family: var(--mono, ui-monospace, monospace); letter-spacing: 0; line-height: 1.4; }
    .vs > button { display: inline-flex; align-items: center; gap: 0.35rem; cursor: pointer;
      font: inherit; font-size: 0.85rem; font-weight: 500; padding: 0.2rem 0.55rem 0.2rem 0.65rem; border-radius: 99px;
      background: var(--accent-soft, #eeeefc); color: var(--accent, #4a4ad8); border: 1px solid transparent; }
    .vs > button:hover, .vs > button[aria-expanded="true"] { border-color: var(--accent, #4a4ad8); }
    .vs > button svg { transition: transform 0.15s; }
    .vs > button[aria-expanded="true"] svg { transform: rotate(180deg); }
    .vs-menu { position: absolute; left: 0; top: calc(100% + 0.4rem); z-index: 40; min-width: 12.5rem; padding: 0.35rem;
      background: var(--bg, #fff); border: 1px solid var(--line, #e2e3ea); border-radius: var(--radius, 10px);
      box-shadow: 0 10px 32px rgb(0 0 0 / 0.2); font-family: var(--sans, system-ui, sans-serif); }
    .vs-menu[hidden] { display: none; }
    .vs-menu a { display: flex; justify-content: space-between; gap: 1rem; padding: 0.4rem 0.6rem; border-radius: 6px;
      color: var(--fg, #1c1d22); text-decoration: none; font-size: 0.9rem; font-weight: 400; white-space: nowrap; }
    .vs-menu a .v { font-family: var(--mono, ui-monospace, monospace); }
    .vs-menu a small { color: var(--fg-faint, #85899a); font-size: 0.78rem; }
    .vs-menu a:hover, .vs-menu a:focus-visible { background: var(--accent-soft, #eeeefc); color: var(--accent, #4a4ad8); outline: none; }
    .vs-menu a[aria-current="page"] { color: var(--accent, #4a4ad8); font-weight: 600; }
    .vs-menu hr { border: 0; border-top: 1px solid var(--line, #e2e3ea); margin: 0.3rem 0.2rem; }
    .vs-banner { background: var(--accent-soft, #eeeefc); color: var(--fg, #1c1d22); border-bottom: 1px solid var(--line, #e2e3ea);
      font-family: var(--sans, system-ui, sans-serif); font-size: 0.92rem; line-height: 1.5; }
    .vs-banner > div { max-width: 64rem; margin: 0 auto; padding: 0.55rem 16px; display: flex; flex-wrap: wrap;
      align-items: center; gap: 0.25rem 0.5rem; }
    .vs-banner .vs { margin-left: 0; }
    .vs-banner .vs > button { background: var(--bg, #fff); border-color: var(--line, #e2e3ea); }
    .vs-banner .vs > button:hover, .vs-banner .vs > button[aria-expanded="true"] { border-color: var(--accent, #4a4ad8); }
    .vs-banner a.vs-latest { color: var(--accent, #4a4ad8); font-weight: 600; }
  `;
  document.head.append(style);

  const el = (tag, attrs = {}, ...children) => {
    const n = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
    n.append(...children);
    return n;
  };

  let count = 0;
  function switcher() {
    const id = `vs-menu-${++count}`;
    const chevron = new DOMParser().parseFromString(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 6" width="9" height="6" aria-hidden="true"><path d="M1 1l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>',
      'image/svg+xml').documentElement;
    const button = el('button', { type: 'button', 'aria-expanded': 'false', 'aria-controls': id,
      'aria-label': `Version ${here}: choose another`, title: 'Choose a version' }, `v${here}`, chevron);
    const menu = el('div', { class: 'vs-menu', id, hidden: '' });
    for (const v of VERSIONS) {
      const a = el('a', { href: hrefFor(v) }, el('span', { class: 'v' }, `v${v.version}`));
      if (v.version === LATEST) a.append(el('small', {}, 'latest'));
      if (v.version === here) a.setAttribute('aria-current', 'page');
      menu.append(a);
    }
    menu.append(el('hr'), el('a', { href: RELEASES + here }, `Release notes for v${here}`, el('small', {}, '↗')));

    const box = el('span', { class: 'vs' }, button, menu);
    const setOpen = (open) => { menu.hidden = !open; button.setAttribute('aria-expanded', String(open)); };
    button.addEventListener('click', () => {
      setOpen(menu.hidden);
      if (!menu.hidden) (menu.querySelector('[aria-current]') || menu.querySelector('a')).focus();
    });
    document.addEventListener('click', (e) => { if (!menu.hidden && !box.contains(e.target)) setOpen(false); });
    box.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && !menu.hidden) { setOpen(false); button.focus(); }
      if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && !menu.hidden) {
        e.preventDefault();
        const links = [...menu.querySelectorAll('a')];
        const i = links.indexOf(document.activeElement);
        links[(i + (e.key === 'ArrowDown' ? 1 : links.length - 1)) % links.length].focus();
      }
    });
    box.addEventListener('focusout', (e) => { if (!box.contains(e.relatedTarget)) setOpen(false); });
    return box;
  }

  // The badge: a link to the release notes until this script runs, so the
  // page still works without it.
  const badge = document.querySelector('.hero a.version');
  if (badge) badge.replaceWith(switcher());

  if (here !== LATEST) {
    const latest = VERSIONS[0];
    const banner = el('div', { class: 'vs-banner', role: 'note' }, el('div', {},
      'You are reading the documentation for ', switcher(),
      ', an older release. ',
      el('a', { class: 'vs-latest', href: hrefFor(latest) }, `Go to the latest, v${LATEST} →`)));
    document.body.prepend(banner);
  }
})();
