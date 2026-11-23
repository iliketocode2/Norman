/* µNorman documentation site: the interpreter in the browser.
 *
 * The whole language runs here as WebAssembly, compiled from the same Rust
 * source as the command-line interpreter. Models answer from scripts, so every
 * example is deterministic, costs nothing and needs no API key -- which is why
 * the examples on this site can actually run rather than being transcripts.
 *
 * The wasm module needs no imports and no wasm-bindgen. Strings cross the
 * boundary as UTF-8 in linear memory, length-prefixed by a little-endian u32.
 * See src/wasm.rs.
 */

const WASM_URL = new URL('./munorman.wasm', import.meta.url);

let modulePromise = null;
let instance = null;

async function loadModule() {
  if (!modulePromise) modulePromise = WebAssembly.compileStreaming(fetch(WASM_URL))
    .catch(async () => WebAssembly.compile(await (await fetch(WASM_URL)).arrayBuffer()));
  return modulePromise;
}

/** A fresh instance. Each program gets one, so no program can affect another. */
async function freshInstance() {
  return WebAssembly.instantiate(await loadModule(), {});
}

/**
 * Load a µNorman program and run its unit tests.
 * @returns {Promise<{transcript: string, passed: number, total: number, ok: boolean}>}
 */
export async function run(source) {
  // A panic aborts the module, so start from a clean instance every time.
  // That also means one runaway example can never wedge the page.
  instance = await freshInstance();
  const w = instance.exports;
  const bytes = new TextEncoder().encode(source);
  const inPtr = w.alloc(bytes.length);
  new Uint8Array(w.memory.buffer, inPtr, bytes.length).set(bytes);
  const outPtr = w.run(inPtr, bytes.length);
  const len = new DataView(w.memory.buffer).getUint32(outPtr, true);
  const json = new TextDecoder().decode(new Uint8Array(w.memory.buffer, outPtr + 4, len));
  w.dealloc(outPtr, len + 4);
  return JSON.parse(json);
}

/* ------------------------------------------------------------- highlighting */

const KEYWORDS = new Set([
  'define', 'val', 'lambda', 'let*', 'if', 'case', 'begin',
  'datatype', 'record', 'grant', 'script', 'under', 'use',
]);

/** The six agentic forms, plus the sugar that expands into them. */
const FORMS = new Set([
  'ask', 'call', 'fail', 'catch', 'budget', 'workflow', 'par', 'and', 'or',
  'retry', 'repair', 'best-of', 'window', 'remaining', 'attempt', 'first-some',
  'show', 'substring', 'string-truncate', 'string-contains?',
  'string-append', 'string-length',
]);

const CHECKS = /^check-(expect|assert|error|fail|within|equiv)$/;

const escape = (s) => s.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));

/**
 * Highlight µNorman source. A small hand-written tokenizer: no grammar for
 * this language exists anywhere, and the lexical structure is tiny (04 §1).
 */
export function highlight(src) {
  let out = '';
  let i = 0;
  const span = (cls, text) => `<span class="tok-${cls}">${escape(text)}</span>`;

  while (i < src.length) {
    const c = src[i];

    // comment to end of line
    if (c === ';') {
      let j = src.indexOf('\n', i);
      if (j === -1) j = src.length;
      out += span('com', src.slice(i, j));
      i = j;
      continue;
    }

    // string, with backslash escapes
    if (c === '"') {
      let j = i + 1;
      while (j < src.length && src[j] !== '"') j += src[j] === '\\' ? 2 : 1;
      j = Math.min(j + 1, src.length);
      out += span('str', src.slice(i, j));
      i = j;
      continue;
    }

    // money ($0.50), duration (10s, 1min, 250ms) and plain numerals
    const rest = src.slice(i);
    const lit = /^(\$\d[\d.]*|-?\d+(?:\.\d+)?(?:ms|s|min|h)?)(?![\w-])/.exec(rest);
    if (lit && /[\d$]/.test(c)) {
      out += span('num', lit[0]);
      i += lit[0].length;
      continue;
    }

    // a bare word: keyword, agentic form, check form, type name, or plain
    const word = /^[^\s()[\]";]+/.exec(rest);
    if (word) {
      const w = word[0];
      let cls = null;
      if (KEYWORDS.has(w)) cls = 'key';
      else if (FORMS.has(w)) cls = 'form';
      else if (CHECKS.test(w)) cls = 'key';
      else if (w.startsWith("'")) cls = 'str';
      else if (/^[A-Z]/.test(w)) cls = 'type';
      out += cls ? span(cls, w) : escape(w);
      i += w.length;
      continue;
    }

    out += escape(c);
    i += 1;
  }
  return out;
}

/* ------------------------------------------------------------ runnable code */

function verdict(result) {
  if (result.loaded === false) return 'Did not load.';
  const errors = result.errors ?? 0;
  // Errors outside the tests (a top-level form that failed, say) fail the
  // run even when every test passes, so they are said alongside the tests.
  const also = errors === 0 ? '' : errors === 1 ? ', but there was 1 error' : `, but there were ${errors} errors`;
  if (result.total === 0) return errors === 0 ? 'Loaded.' : `Loaded${also}.`;
  const tests =
    result.passed === result.total
      ? result.total === 1 ? 'The only test passed' : `All ${result.total} tests passed`
      : `${result.passed} of ${result.total} tests passed`;
  return `${tests}${also}.`;
}

/** Show a result in `box`, an element with class `result` or `out-body`. */
function render(box, result) {
  const pass = result.ok && result.total > 0;
  box.classList.remove('pass', 'fail');
  box.classList.add('show', pass ? 'pass' : 'fail');
  const lines = result.transcript.trimEnd();
  box.innerHTML = `<span class="verdict">${escape(verdict(result))}</span>${escape(lines)}`;
}

/**
 * Wire up every `.example` on the page: highlight its source, and give its Run
 * button a result panel.
 */
export function activateExamples() {
  for (const ex of document.querySelectorAll('.example')) {
    const code = ex.querySelector('code');
    const source = code.textContent.replace(/^\n/, '').trimEnd();
    code.innerHTML = highlight(source);

    const box = ex.querySelector('.result');
    const button = ex.querySelector('button.run');
    if (!button || !box) continue;

    button.addEventListener('click', async () => {
      button.disabled = true;
      const label = button.textContent;
      button.textContent = 'Running…';
      try {
        render(box, await run(source));
      } catch (err) {
        box.classList.add('show', 'fail');
        box.innerHTML = `<span class="verdict">The interpreter could not start.</span>${escape(String(err))}`;
      } finally {
        button.disabled = false;
        button.textContent = label;
      }
    });

    const open = ex.querySelector('button.open');
    if (open) {
      open.addEventListener('click', () => {
        sessionStorage.setItem('munorman.playground', source);
        location.href = 'playground.html';
      });
    }

    const copy = ex.querySelector('button.copy');
    if (copy) {
      copy.addEventListener('click', async () => {
        await navigator.clipboard.writeText(source);
        const was = copy.textContent;
        copy.textContent = 'Copied';
        setTimeout(() => { copy.textContent = was; }, 1200);
      });
    }
  }
}

/* ------------------------------------------------------------- playground */

/** A textarea over a highlighted <pre>: a real editor without a dependency. */
export function activatePlayground(examples) {
  const area = document.getElementById('src');
  const view = document.getElementById('view');
  const out = document.getElementById('out');
  const runBtn = document.getElementById('run');
  const picker = document.getElementById('examples');
  if (!area) return;

  // `view` is the <pre>, which is the element that scrolls. Setting scrollTop
  // on the <code> inside it would do nothing at all, since only the <pre> has
  // a scrolling box -- that was the original bug.
  const sync = () => {
    view.scrollTop = area.scrollTop;
    view.scrollLeft = area.scrollLeft;
  };

  const paint = () => {
    // Replacing the content resets the <pre>'s scroll, so re-sync right after.
    // The trailing newline keeps the final line reachable when scrolled down.
    view.innerHTML = highlight(area.value) + '\n';
    sync();
  };

  area.addEventListener('input', paint);
  area.addEventListener('scroll', sync, { passive: true });
  // Dragging the editor's bottom edge changes what is visible, not the text.
  new ResizeObserver(sync).observe(area);

  /**
   * Replace the selection, keeping the browser's native undo history.
   * Assigning to `.value` would wipe it, so a single Ctrl+Z after pressing
   * Tab used to discard the whole program.
   */
  const insert = (text) => {
    area.focus();
    const done = text
      ? document.execCommand?.('insertText', false, text)
      : document.execCommand?.('delete');
    if (!done) {
      const { selectionStart: s, selectionEnd: e } = area;
      area.setRangeText(text, s, e, 'end');
    }
  };

  const INDENT = '  ';

  area.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      runBtn.click();
      return;
    }

    // Escape hands the keyboard back, so Tab can still leave the editor for
    // anyone navigating the page without a mouse.
    if (e.key === 'Escape') {
      area.blur();
      return;
    }

    if (e.key !== 'Tab' || e.altKey || e.ctrlKey || e.metaKey) return;
    e.preventDefault();

    const { selectionStart: s, selectionEnd: t } = area;
    const value = area.value;
    const lineStart = value.lastIndexOf('\n', s - 1) + 1;

    if (e.shiftKey) {
      // Outdent: drop up to two spaces from the start of this line.
      const take = value.slice(lineStart, lineStart + INDENT.length).match(/^ {1,2}/);
      if (!take) return;
      area.setSelectionRange(lineStart, lineStart + take[0].length);
      insert('');
      const back = (p) => Math.max(lineStart, p - take[0].length);
      area.setSelectionRange(back(s), back(t));
    } else {
      insert(INDENT);
    }
    paint();
  });

  for (const [name] of Object.entries(examples)) {
    const opt = document.createElement('option');
    opt.value = name;
    opt.textContent = name;
    picker.append(opt);
  }

  picker.addEventListener('change', () => {
    area.value = examples[picker.value];
    paint();
    out.className = 'out-body';
    out.textContent = 'Not run yet.';
  });

  runBtn.addEventListener('click', async () => {
    runBtn.disabled = true;
    runBtn.textContent = 'Running…';
    out.className = 'out-body';
    out.textContent = 'Running…';
    try {
      const result = await run(area.value);
      out.className = 'out-body ' + (result.ok && result.total > 0 ? 'pass' : 'fail');
      out.innerHTML = `<span class="verdict">${escape(verdict(result))}</span>\n${escape(result.transcript.trimEnd())}`;
    } catch (err) {
      out.className = 'out-body fail';
      out.textContent = 'The interpreter could not start: ' + err;
    } finally {
      runBtn.disabled = false;
      runBtn.textContent = 'Run  ⌃⏎';
    }
  });

  const saved = sessionStorage.getItem('munorman.playground');
  sessionStorage.removeItem('munorman.playground');
  area.value = saved || examples[Object.keys(examples)[0]];
  paint();
}

/* ------------------------------------------------------------------ chrome */

export function activateChrome() {
  const root = document.documentElement;
  const saved = (() => { try { return localStorage.getItem('munorman.theme'); } catch { return null; } })();
  if (saved) root.dataset.theme = saved;

  const button = document.querySelector('button.theme');
  if (button) {
    const dark = () => (root.dataset.theme
      ? root.dataset.theme === 'dark'
      : matchMedia('(prefers-color-scheme: dark)').matches);
    const label = () => { button.textContent = dark() ? '☀' : '☾'; };
    label();
    button.addEventListener('click', () => {
      root.dataset.theme = dark() ? 'light' : 'dark';
      try { localStorage.setItem('munorman.theme', root.dataset.theme); } catch { /* private mode */ }
      label();
    });
  }

  // A link beside every heading that has an id.
  for (const h of document.querySelectorAll('h2[id], h3[id]')) {
    const a = document.createElement('a');
    a.className = 'anchor';
    a.href = '#' + h.id;
    a.textContent = '#';
    a.setAttribute('aria-label', 'Link to this section');
    h.append(a);
  }

  activateSearch();
  activateExport();
}

/* ------------------------------------------------------------------ search */

/**
 * The search box in the header. The index is docs/search-index.js, generated
 * from the pages by scripts/docs-assets.mjs (docs/verify.mjs fails if it is
 * stale), and loaded the first time the box opens.
 */
function activateSearch() {
  const opener = document.querySelector('button.search-open');
  if (!opener) return;

  const mac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  opener.title = `Search the docs (/ or ${mac ? '⌘' : 'Ctrl+'}K)`;

  let index = null;
  const load = () => (index ??= import('./search-index.js').then(({ SEARCH_INDEX }) => SEARCH_INDEX.map((e) => ({
    ...e, tl: e.t.toLowerCase(), kl: (e.k ?? '').toLowerCase(), sl: (e.s ?? '').toLowerCase(), xl: e.x.toLowerCase(),
  }))).catch((err) => { index = null; throw err; }));

  let dialog = null;
  let input = null;
  let list = null;
  let status = null;
  let results = [];
  let active = -1;

  const build = () => {
    dialog = document.createElement('dialog');
    dialog.className = 'search';
    dialog.setAttribute('aria-label', 'Search the documentation');
    dialog.innerHTML = `
      <div class="search-box">
        <svg viewBox="0 0 16 16" width="17" height="17" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" d="M7 12.2A5.2 5.2 0 1 0 7 1.8a5.2 5.2 0 0 0 0 10.4zM10.9 10.9l3.6 3.6"/></svg>
        <input type="search" placeholder="Search the docs" autocomplete="off" spellcheck="false"
               role="combobox" aria-expanded="false" aria-controls="search-results" aria-autocomplete="list"
               aria-label="Search the docs">
        <kbd>Esc</kbd>
      </div>
      <p class="search-status" role="status"></p>
      <ul class="search-results" id="search-results" role="listbox" aria-label="Results"></ul>`;
    document.body.append(dialog);
    input = dialog.querySelector('input');
    list = dialog.querySelector('ul');
    status = dialog.querySelector('.search-status');

    input.addEventListener('input', update);
    input.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (results.length) select((active + (e.key === 'ArrowDown' ? 1 : -1) + results.length) % results.length);
      } else if (e.key === 'Enter' && !e.isComposing) {
        e.preventDefault();
        if (results.length) go(results[Math.max(active, 0)]);
      }
    });
    // Escape always closes, even with text in the box: a search input would
    // otherwise spend the first press clearing itself.
    dialog.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') { e.preventDefault(); dialog.close(); }
    });
    // A click on the backdrop lands on the dialog itself.
    dialog.addEventListener('click', (e) => { if (e.target === dialog) dialog.close(); });
    list.addEventListener('mousemove', (e) => {
      const li = e.target.closest('li[data-i]');
      if (li && +li.dataset.i !== active) select(+li.dataset.i);
    });
    list.addEventListener('click', (e) => {
      const li = e.target.closest('li[data-i]');
      if (!li || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
      e.preventDefault();
      go(results[+li.dataset.i]);
    });
  };

  const open = () => {
    if (!dialog) build();
    if (!dialog.open) dialog.showModal();
    input.focus();
    input.select();
    if (!index) {
      status.textContent = 'Loading…';
      load().then(update, () => { status.textContent = 'The search index could not be loaded.'; });
    }
  };

  opener.addEventListener('click', open);
  document.addEventListener('keydown', (e) => {
    const editable = e.target.closest?.('input, textarea, select, [contenteditable]:not([contenteditable="false"])');
    if ((e.key === 'k' || e.key === 'K') && (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      open();
    } else if (e.key === '/' && !e.ctrlKey && !e.metaKey && !e.altKey && !editable) {
      e.preventDefault();
      open();
    }
  });

  function select(i) {
    const items = list.children;
    if (items[active]) items[active].setAttribute('aria-selected', 'false');
    active = i;
    if (items[i]) {
      items[i].setAttribute('aria-selected', 'true');
      items[i].scrollIntoView({ block: 'nearest' });
      input.setAttribute('aria-activedescendant', items[i].id);
    } else {
      input.removeAttribute('aria-activedescendant');
    }
  }

  async function update() {
    const query = input.value.trim().toLowerCase();
    const terms = query.split(/\s+/).filter(Boolean);
    const entries = await load().catch(() => null);
    if (!entries || input.value.trim().toLowerCase() !== query) return;   // a newer keystroke wins
    results = terms.length ? rank(entries, query, terms) : [];
    list.innerHTML = results.map((e, i) => `
      <li role="option" id="search-result-${i}" data-i="${i}" aria-selected="false">
        <a href="${escape(e.u)}" tabindex="-1">
          <span class="sr-where">${escape(e.p + (e.s ? ' › ' + e.s : ''))}</span>
          <span class="sr-title">${mark(e.t, terms)}</span>
          ${e.x ? `<span class="sr-text">${snippet(e.x, terms)}</span>` : ''}
        </a>
      </li>`).join('');
    input.setAttribute('aria-expanded', String(results.length > 0));
    status.textContent = !terms.length ? 'Search every heading and section on the site.'
      : results.length ? '' : `Nothing matches “${input.value.trim()}”.`;
    active = -1;
    select(results.length ? 0 : -1);
  }

  /** Go to a result. On the page already open, that is just a scroll. */
  function go(entry) {
    dialog.close();
    const url = new URL(entry.u, location.href);
    const page = (p) => p.replace(/(\/index)?(\.html)?$/, '').replace(/\/$/, '') || '/';
    if (page(url.pathname) !== page(location.pathname)) { location.href = url.href; return; }
    const target = url.hash && document.getElementById(decodeURIComponent(url.hash.slice(1)));
    if (url.hash && url.hash !== location.hash) location.hash = url.hash;
    else if (target) target.scrollIntoView();
    else if (!url.hash) { history.pushState(null, '', location.pathname); window.scrollTo(0, 0); }
    // On a phone the docs sidebar is a dropdown above the content; don't leave it open.
    const toc = document.querySelector('details.toc');
    if (toc && window.innerWidth < 992) toc.open = false;
  }
}

/** Every term must appear somewhere; headings count far more than body text. */
function rank(entries, query, terms) {
  const scored = [];
  for (const e of entries) {
    let score = 0;
    let all = true;
    for (const t of terms) {
      const inTitle = e.tl.includes(t);
      const inShort = e.kl.includes(t);
      const inSection = e.sl.includes(t);
      const hits = count(e.xl, t);
      if (!inTitle && !inShort && !inSection && !hits) { all = false; break; }
      if (e.tl === t) score += 30;
      else if (startsWord(e.tl, t)) score += 12;
      else if (inTitle) score += 3;
      if (inShort) score += startsWord(e.kl, t) ? 10 : 5;
      if (inSection) score += 2;
      score += Math.min(hits, 6) * (startsWord(e.xl, t) ? 1 : 0.5);
    }
    if (!all) continue;
    if (terms.length > 1 && e.tl.includes(query)) score += 15;
    if (e.p === 'Docs') score += 1.5;     // the reference is what people search for
    if (e.l === 1) score -= 3;            // a whole page is rarely the best answer
    scored.push({ e, score });
  }
  return scored.sort((a, b) => b.score - a.score).slice(0, 12).map((s) => s.e);
}

function count(hay, t) {
  let n = 0;
  for (let i = hay.indexOf(t); i !== -1; i = hay.indexOf(t, i + t.length)) n += 1;
  return n;
}

/** Does `t` occur in `hay` at the start of a word? */
function startsWord(hay, t) {
  for (let i = hay.indexOf(t); i !== -1; i = hay.indexOf(t, i + 1)) {
    if (i === 0 || !/[\p{L}\p{N}]/u.test(hay[i - 1])) return true;
  }
  return false;
}

/** Escaped HTML for `text`, with every term wrapped in <mark>. */
function mark(text, terms) {
  const re = new RegExp(terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|'), 'gi');
  let out = '';
  let last = 0;
  for (const m of text.matchAll(re)) {
    out += escape(text.slice(last, m.index)) + `<mark>${escape(m[0])}</mark>`;
    last = m.index + m[0].length;
  }
  return out + escape(text.slice(last));
}

/** About two lines of the section, around the first term found in it. */
function snippet(text, terms) {
  const lower = text.toLowerCase();
  const hits = terms.map((t) => lower.indexOf(t)).filter((i) => i >= 0);
  const at = hits.length ? Math.min(...hits) : 0;
  let start = Math.max(0, at - 50);
  if (start > 0) start = text.indexOf(' ', start) + 1 || start;
  let end = Math.min(text.length, start + 170);
  if (end < text.length) end = text.lastIndexOf(' ', end) > start ? text.lastIndexOf(' ', end) : end;
  return (start > 0 ? '… ' : '') + mark(text.slice(start, end), terms) + (end < text.length ? ' …' : '');
}

/* ---------------------------------------------------------- Markdown export */

/**
 * The ".md" button beside the GitHub link: the language reference as
 * Markdown (docs/munorman-reference.md, generated from docs.html), to
 * download or copy into an agent's context.
 */
function activateExport() {
  const box = document.querySelector('.md-export');
  if (!box) return;
  const toggle = box.querySelector('button.md-float');
  const menu = box.querySelector('.md-menu');
  const link = menu.querySelector('a[download]');
  const copy = menu.querySelector('button.md-copy');

  let text = null;     // a promise of the file
  let ready = null;    // the file, once it has arrived
  const fetchText = () => (text ??= fetch(link.href)
    .then((r) => { if (!r.ok) throw new Error(`HTTP ${r.status}`); return r.text(); })
    .then((t) => (ready = t))
    .catch((err) => { text = null; throw err; }));

  const setOpen = (open) => {
    menu.hidden = !open;
    toggle.setAttribute('aria-expanded', String(open));
    // Fetch now, so that Copy can write to the clipboard straight away:
    // some browsers refuse once the click is a network round trip old.
    if (open) fetchText().catch(() => {});
  };

  toggle.addEventListener('click', () => setOpen(menu.hidden));
  document.addEventListener('click', (e) => { if (!menu.hidden && !box.contains(e.target)) setOpen(false); });
  box.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !menu.hidden) { setOpen(false); toggle.focus(); }
  });
  link.addEventListener('click', () => setTimeout(() => setOpen(false), 0));

  const label = copy.textContent;
  copy.addEventListener('click', async () => {
    try {
      if (ready !== null) {
        await navigator.clipboard.writeText(ready);
      } else if (window.ClipboardItem && navigator.clipboard.write) {
        // Hand the clipboard a promise, which keeps the click's permission.
        const blob = fetchText().then((t) => new Blob([t], { type: 'text/plain' }));
        await navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })]);
      } else {
        await navigator.clipboard.writeText(await fetchText());
      }
      copy.textContent = 'Copied';
    } catch {
      copy.textContent = 'Could not copy; use Download';
    }
    setTimeout(() => { copy.textContent = label; }, 1800);
  });
}

/* ------------------------------------------------------------ reference TOC */

/**
 * Build the sidebar from the page's own headings, and keep the current
 * section marked as you scroll. Generating it from the document means the
 * two can never drift apart.
 */
export function activateToc() {
  const toc = document.querySelector('.toc');
  const body = document.querySelector('.doc-body');
  if (!toc || !body) return;

  const list = document.createElement('ol');
  const links = new Map();

  for (const h of body.querySelectorAll('h2[id], h3[id]')) {
    const li = document.createElement('li');
    if (h.tagName === 'H3') li.className = 'sub';
    const a = document.createElement('a');
    a.href = '#' + h.id;
    a.textContent = h.dataset.short || h.textContent.replace(/#$/, '');
    li.append(a);
    list.append(li);
    links.set(h.id, a);
  }
  toc.append(list);

  // Mark the heading nearest the top of the viewport.
  const seen = new Map();
  const observer = new IntersectionObserver((entries) => {
    for (const e of entries) seen.set(e.target.id, e);
    let best = null;
    for (const e of seen.values()) {
      if (!e.isIntersecting) continue;
      if (!best || e.boundingClientRect.top < best.boundingClientRect.top) best = e;
    }
    if (!best) return;
    for (const a of links.values()) a.classList.remove('here');
    const a = links.get(best.target.id);
    if (a) {
      a.classList.add('here');
      // Keep the active link in view in a long sidebar, without scrolling the page.
      if (toc.scrollHeight > toc.clientHeight) {
        const top = a.offsetTop - toc.clientHeight / 2;
        toc.scrollTo({ top, behavior: 'smooth' });
      }
    }
  }, { rootMargin: '-72px 0px -70% 0px', threshold: 0 });

  for (const id of links.keys()) observer.observe(document.getElementById(id));

  // On a phone the sidebar is a dropdown; choosing a section closes it.
  toc.addEventListener('click', (e) => {
    if (e.target.tagName === 'A' && toc.tagName === 'DETAILS' && window.innerWidth < 992) {
      toc.open = false;
    }
  });
}
