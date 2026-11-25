/* Check the documentation site.
 *
 *   node docs/verify.mjs
 *
 * Runs every example shown on the site, and every starter program in the
 * playground, through the interpreter the site actually ships. A broken
 * example fails here rather than in front of a reader. Also asserts the
 * structural invariants the playground editor depends on.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const bytes = fs.readFileSync(path.join(here, 'munorman.wasm'));
const mod = await WebAssembly.compile(bytes);

async function run(source) {
  const { exports: w } = await WebAssembly.instantiate(mod, {});
  const enc = new TextEncoder().encode(source);
  const p = w.alloc(enc.length);
  new Uint8Array(w.memory.buffer, p, enc.length).set(enc);
  const r = w.run(p, enc.length);
  const n = new DataView(w.memory.buffer).getUint32(r, true);
  const json = new TextDecoder().decode(new Uint8Array(w.memory.buffer, r + 4, n));
  w.dealloc(r, n + 4);
  return JSON.parse(json);
}

const unescape = (s) => s
  .replace(/&lt;/g, '<').replace(/&gt;/g, '>')
  .replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&');

let failures = 0;
let checked = 0;

function report(name, problems) {
  checked += 1;
  if (problems.length) {
    failures += 1;
    console.error(`FAIL  ${name}\n  - ${problems.join('\n  - ')}`);
  } else {
    console.log(`ok    ${name}`);
  }
}

/* The playground editor is a transparent textarea laid over a highlighted
 * <pre>. It is easy to break in ways no example would catch. These are the
 * invariants it depends on; the first is the bug that shipped in v0.1.0,
 * where the script scrolled a <code> element, which has no scrolling box, so
 * the highlighted layer never moved while the textarea scrolled underneath. */
function checkEditor() {
  const html = fs.readFileSync(path.join(here, 'playground.html'), 'utf8');
  const css = fs.readFileSync(path.join(here, 'style.css'), 'utf8');
  const problems = [];

  if (!/<pre id="view"/.test(html)) {
    problems.push('#view must be the <pre>: that is the element with a scrolling box');
  }
  if (/<code id="view"/.test(html)) {
    problems.push('#view must not be a <code>: setting scrollTop on it does nothing');
  }
  if (!/<textarea[^>]*\bwrap="off"/.test(html)) {
    problems.push('the textarea needs wrap="off", so long lines scroll instead of wrapping');
  }

  const pre = /^\.editor > pre \{([^}]*)\}/m.exec(css);
  const area = /^\.editor > textarea \{([^}]*)\}/m.exec(css);
  if (!pre || !/overflow:\s*hidden/.test(pre[1])) {
    problems.push('.editor > pre must be overflow:hidden, or its scrollbar misaligns the text');
  }
  if (!area || !/overflow:\s*auto/.test(area[1])) {
    problems.push('.editor > textarea must be overflow:auto: it is the only scroller');
  }

  // Anything affecting glyph position has to be set on both layers at once.
  const shared = /^\.editor > pre, \.editor > textarea \{([^}]*)\}/m.exec(css);
  for (const prop of ['font-family', 'font-size', 'line-height', 'padding', 'tab-size', 'white-space']) {
    if (!shared || !new RegExp(prop + ':').test(shared[1])) {
      problems.push(`${prop} must be set on both layers together, or the highlighting drifts`);
    }
  }

  // The script has to keep the two layers in step.
  const js = fs.readFileSync(path.join(here, 'norman.js'), 'utf8');
  for (const needed of ['scrollTop = area.scrollTop', 'scrollLeft = area.scrollLeft']) {
    if (!js.includes(needed)) problems.push(`norman.js must copy ${needed.split(' ')[0]} to the <pre>`);
  }

  report('playground editor', problems);
}

checkEditor();

/* The Markdown export and the search index are generated from the pages and
 * committed, since Pages has no build step. Each must match what the
 * generator makes from the pages as they are now. Line endings are ignored:
 * a Windows checkout may have turned LF into CRLF. */
async function checkGenerated() {
  const { generate } = await import(new URL('../scripts/docs-assets.mjs', import.meta.url));
  for (const [file, expected] of Object.entries(generate())) {
    const full = path.join(here, file);
    const actual = fs.existsSync(full) ? fs.readFileSync(full, 'utf8').replace(/\r\n/g, '\n') : null;
    report(`generated: docs/${file}`, actual === expected ? [] : [
      actual === null ? 'missing' : 'out of date with the pages it is generated from',
      'run `node scripts/docs-assets.mjs` and commit the result',
    ]);
  }
}

await checkGenerated();

/* Claims known to have gone stale. Agents are given the reference as context,
 * so a false sentence there becomes a wrong program. Each entry is a pattern
 * that was once true and isn't now, with what replaced it. Only the pages
 * that describe the language as it is today are scanned: the numbered design
 * documents record history, and may say what was true when they were written
 * (design/13, Phase 0). Add a pattern whenever a release falsifies a claim. */
const STALE = [
  [/planned type and effect system/i, 'the type and effect checker is built (design/12)'],
  [/\bdraft, not built\b[^|]*type and effect/i, 'the type and effect checker is built (design/12)'],
  [/everything else about types is a\s+run-time check/i, 'types are checked before a program runs'],
  [/askability is checked when the\s+(<code>)?`?ask`?(<\/code>)? runs/i, 'askability is checked before a program runs'],
  [/\bsix agentic forms\b/i, 'there are seven: consult is one'],
  [/enforced (\*\*)?when the program (<em>)?runs/i, 'types are checked before a program runs; only budgets and deadlines are enforced while it runs'],
];

function checkStale() {
  const root = path.join(here, '..');
  const files = [
    'README.md', 'design/README.md',
    'docs/index.html', 'docs/docs.html', 'docs/demo.html', 'docs/playground.html',
    'docs/munorman-reference.md',
  ];
  const problems = [];
  for (const file of files) {
    const full = path.join(root, file);
    if (!fs.existsSync(full)) continue;
    const text = fs.readFileSync(full, 'utf8');
    for (const [pattern, truth] of STALE) {
      const m = text.match(pattern);
      if (m) {
        const line = text.slice(0, m.index).split('\n').length;
        problems.push(`${file}:${line}: "${m[0]}" is stale: ${truth}`);
      }
    }
  }
  report('no stale claims about the language', problems);
}

checkStale();

// Every starter program in the playground.
const { EXAMPLES } = await import(new URL('./examples.js', import.meta.url));
for (const [name, source] of Object.entries(EXAMPLES)) {
  const res = await run(source);
  report(`example: ${name}  (${res.total} tests)`, res.ok && res.total > 0 ? [] : [res.transcript.trim()]);
}

// Every runnable block shown on a page. Only blocks inside a `.example`
// container are programs; a `<pre class="syntax">` on the reference page is a
// grammar, and running it would be meaningless.
for (const file of ['index.html', 'demo.html', 'docs.html', 'playground.html']) {
  const full = path.join(here, file);
  if (!fs.existsSync(full)) continue;
  const html = fs.readFileSync(full, 'utf8');
  for (const m of html.matchAll(/<div class="example">[\s\S]*?class="title">([^<]*)<[\s\S]*?<pre><code>([\s\S]*?)<\/code><\/pre>/g)) {
    const name = m[1].trim() || file;
    const source = unescape(m[2]);
    const res = await run(source);
    report(`${name}  (${res.total} tests)`, res.ok && res.total > 0 ? [] : [res.transcript.trim()]);
  }
}

console.log(`\n${checked - failures} of ${checked} checks pass.`);
process.exit(failures ? 1 : 0);
