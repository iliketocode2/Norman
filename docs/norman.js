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
  if (result.total === 0) return result.ok ? 'Loaded.' : 'Did not load.';
  if (result.ok) return result.total === 1 ? 'The only test passed.' : `All ${result.total} tests passed.`;
  return `${result.passed} of ${result.total} tests passed.`;
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
}
