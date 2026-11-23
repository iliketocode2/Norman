/* Generate the documentation site's derived files from its pages.
 *
 *   node scripts/docs-assets.mjs
 *
 * Writes two files into docs/, both committed, because GitHub Pages serves
 * docs/ straight from the branch with no build step:
 *
 *   docs/munorman-reference.md  the language reference (docs/docs.html) as
 *                               Markdown, for pasting into an agent's context;
 *                               the "Export as .md" button serves this file.
 *   docs/search-index.js        every heading on the site with its section's
 *                               text, for the search box in the header.
 *
 * Re-run it whenever a page changes. `node docs/verify.mjs` imports
 * `generate` from here and fails if either committed file is out of date.
 *
 * No dependencies: the pages are hand-written, well-formed HTML, so a small
 * parser is enough, and it fails loudly on anything it does not understand
 * (an unknown entity, say) rather than producing something subtly wrong.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const DOCS = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'docs');

/** The pages search covers, in the order results from equal matches appear. */
const PAGES = [
  { file: 'docs.html', url: 'docs.html', label: 'Docs' },
  { file: 'index.html', url: './', label: 'About' },
  { file: 'demo.html', url: 'demo.html', label: 'Demo' },
  { file: 'playground.html', url: 'playground.html', label: 'Playground' },
];

export const OUTPUTS = ['munorman-reference.md', 'search-index.js'];

/* -------------------------------------------------------------- parsing */

const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
const RAW = new Set(['script', 'style']);

const ENTITIES = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: '\u00a0',
  rsquo: '\u2019', lsquo: '\u2018', ldquo: '\u201c', rdquo: '\u201d',
  mdash: '\u2014', ndash: '\u2013', hellip: '\u2026', middot: '\u00b7',
  times: '\u00d7', rarr: '\u2192', larr: '\u2190', copy: '\u00a9',
};

export function decode(s) {
  return s.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z][a-z0-9]*);/gi, (all, name) => {
    if (name[0] === '#') {
      const code = name[1] === 'x' || name[1] === 'X' ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
      return String.fromCodePoint(code);
    }
    if (!(name in ENTITIES)) throw new Error(`unknown HTML entity ${all}: add it to ENTITIES in scripts/docs-assets.mjs`);
    return ENTITIES[name];
  });
}

/** Parse HTML into a tree of { tag, attrs, children } and { text } nodes. */
export function parse(html) {
  html = html.replace(/\r\n?/g, '\n');
  const root = { tag: '#root', attrs: {}, children: [] };
  const stack = [root];
  const top = () => stack[stack.length - 1];
  const re = /<!--[\s\S]*?-->|<![^>]*>|<\/([a-zA-Z][\w-]*)\s*>|<([a-zA-Z][\w-]*)((?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)*)\s*\/?>/g;
  let last = 0;
  let m;
  while ((m = re.exec(html))) {
    if (m.index > last) top().children.push({ text: decode(html.slice(last, m.index)) });
    last = re.lastIndex;
    if (m[1]) {
      const tag = m[1].toLowerCase();
      const at = stack.map((n) => n.tag).lastIndexOf(tag);
      if (at > 0) stack.length = at;
    } else if (m[2]) {
      const tag = m[2].toLowerCase();
      const attrs = {};
      for (const a of m[3].matchAll(/([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g)) {
        attrs[a[1].toLowerCase()] = decode(a[2] ?? a[3] ?? a[4] ?? '');
      }
      const node = { tag, attrs, children: [] };
      top().children.push(node);
      if (RAW.has(tag)) {
        const end = html.indexOf(`</${tag}`, last);
        node.children.push({ text: html.slice(last, end) });
        re.lastIndex = last = html.indexOf('>', end) + 1;
      } else if (!VOID.has(tag)) {
        stack.push(node);
      }
    }
  }
  if (last < html.length) top().children.push({ text: decode(html.slice(last)) });
  return root;
}

const classes = (n) => (n.attrs?.class ?? '').split(/\s+/).filter(Boolean);
const hasClass = (n, c) => classes(n).includes(c);

function* walk(n) {
  yield n;
  for (const c of n.children ?? []) yield* walk(c);
}

const find = (n, pred) => { for (const x of walk(n)) if (x.tag && pred(x)) return x; return null; };
const text = (n) => (n.text !== undefined ? n.text : n.children.map(text).join(''));
const squash = (s) => s.replace(/\s+/g, ' ').trim();

/* --------------------------------------------------------------- Markdown */

/** Site chrome: never part of the content. */
const isChrome = (n) => ['nav', 'button', 'script', 'style', 'template'].includes(n.tag)
  || hasClass(n, 'tools') || hasClass(n, 'result') || hasClass(n, 'anchor');

function code(s) {
  s = s.replace(/\s+/g, ' ').trim();
  const runs = s.match(/`+/g) ?? [];
  const ticks = '`'.repeat(Math.max(0, ...runs.map((r) => r.length)) + 1);
  const pad = s.startsWith('`') || s.endsWith('`') ? ' ' : '';
  return ticks + pad + s + pad + ticks;
}

function fence(body, info = '') {
  body = body.replace(/^\n/, '').replace(/\s+$/, '');
  const runs = body.match(/`{3,}/g) ?? [];
  const ticks = '`'.repeat(Math.max(2, ...runs.map((r) => r.length)) + 1);
  return `${ticks}${info}\n${body}\n${ticks}`;
}

const escapeMd = (s) => s.replace(/[\\`*_[\]]/g, '\\$&').replace(/<(?=[a-zA-Z/!])/g, '\\<');

/** Inline content as Markdown, on one line. `base` resolves relative links. */
function inline(n, base) {
  let out = '';
  for (const c of n.children) {
    if (c.text !== undefined) { out += escapeMd(c.text.replace(/\u00a0/g, ' ').replace(/\s+/g, ' ')); continue; }
    if (isChrome(c)) continue;
    switch (c.tag) {
      case 'code': case 'kbd': out += code(text(c)); break;
      case 'strong': case 'b': out += wrap(inline(c, base), '**'); break;
      case 'em': case 'i': out += wrap(inline(c, base), '*'); break;
      case 'br': out += ' '; break;
      // Outside a grammar block, the colours a grammar uses are explained in
      // prose ("words in this colour are literal"), so keep them visible.
      case 'span':
        out += hasClass(c, 'lit') ? wrap(inline(c, base), '**')
          : hasClass(c, 'meta') ? wrap(inline(c, base), '*') : inline(c, base);
        break;
      case 'a': {
        const href = c.attrs.href ? new URL(c.attrs.href, base).href : '';
        const label = inline(c, base).trim();
        out += href ? `[${label}](${href})` : label;
        break;
      }
      default: out += inline(c, base);
    }
  }
  return out.replace(/ {2,}/g, ' ');
}

/** `**x**`, keeping surrounding spaces outside the markers. */
function wrap(s, mark) {
  const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(s);
  return m[2] ? `${m[1]}${mark}${m[2]}${mark}${m[3]}` : s;
}

/** Split a signature such as "cons · list" into separately quoted pieces. */
const codeList = (s) => squash(s).split(' · ').map(code).join(' · ');

function table(n, base) {
  const rows = [];
  let head = null;
  for (const x of walk(n)) {
    if (x.tag !== 'tr') continue;
    const cells = x.children.filter((c) => c.tag === 'td' || c.tag === 'th').map((c) => {
      const s = hasClass(c, 'sig') ? codeList(text(c)) : inline(c, base).trim();
      return s.replace(/\|/g, '\\|');
    });
    if (!head && x.children.some((c) => c.tag === 'th')) head = cells; else rows.push(cells);
  }
  const width = Math.max(head?.length ?? 0, ...rows.map((r) => r.length));
  const line = (cells) => `| ${Array.from({ length: width }, (_, i) => cells[i] ?? '').join(' | ')} |`;
  return [line(head ?? []), `|${' --- |'.repeat(width)}`, ...rows.map(line)].join('\n');
}

/** Block content as a list of Markdown blocks. */
function blocks(n, base) {
  const out = [];
  for (const c of n.children) {
    if (c.text !== undefined) {
      if (c.text.trim()) out.push(escapeMd(squash(c.text)));
      continue;
    }
    if (isChrome(c)) continue;
    const h = /^h([1-6])$/.exec(c.tag);
    if (h) { out.push(`${'#'.repeat(+h[1])} ${inline(c, base).trim()}`); continue; }
    switch (c.tag) {
      case 'p': out.push(inline(c, base).trim()); break;
      case 'pre': out.push(fence(text(c))); break;
      case 'table': out.push(table(c, base)); break;
      case 'ul': case 'ol': {
        let i = 0;
        out.push(c.children.filter((li) => li.tag === 'li')
          .map((li) => `${c.tag === 'ol' ? `${++i}.` : '-'} ${inline(li, base).trim()}`).join('\n'));
        break;
      }
      case 'dl': {
        const items = [];
        for (const d of c.children) {
          if (d.tag === 'dt') items.push(`- ${codeList(text(d))}`);
          else if (d.tag === 'dd') items[items.length - 1] += `\n\n  ${inline(d, base).trim()}`;
        }
        out.push(items.join('\n\n'));
        break;
      }
      case 'div': case 'section': case 'article': case 'main': case 'details': case 'summary':
        if (hasClass(c, 'example')) {
          const title = squash(text(find(c, (x) => hasClass(x, 'title')) ?? { text: '' }));
          if (title) out.push(`Example, ${code(title)}:`);
          out.push(fence(text(find(c, (x) => x.tag === 'pre')), 'nrm'));
        } else if (hasClass(c, 'note')) {
          const label = find(c, (x) => hasClass(x, 'label'));
          const body = blocks({ children: c.children.filter((x) => x !== label) }, base);
          const parts = label ? [`**${inline(label, base).trim()}**`, ...body] : body;
          out.push(parts.join('\n\n').split('\n').map((l) => (l ? `> ${l}` : '>')).join('\n'));
        } else {
          out.push(...blocks(c, base));
        }
        break;
      default: {
        const s = inline(c, base).trim();
        if (s) out.push(s);
      }
    }
  }
  return out;
}

export function referenceMarkdown(html) {
  const doc = parse(html);
  const canonical = find(doc, (x) => x.tag === 'link' && x.attrs.rel === 'canonical')?.attrs.href;
  if (!canonical) throw new Error('docs.html needs <link rel="canonical"> to resolve its links');
  const article = find(doc, (x) => hasClass(x, 'doc-body'));
  if (!article) throw new Error('docs.html has no .doc-body');
  const body = blocks(article, canonical);
  if (body[0]?.startsWith('# ')) body[0] = '# µNorman language reference';
  return [
    '<!-- Generated from docs/docs.html by scripts/docs-assets.mjs. Do not edit by hand. -->',
    body[0],
    `> This is the reference for µNorman, a small programming language for AI agents, from <${canonical}>, ` +
      'converted to Markdown so it can be given to an agent as context. Examples are µNorman source (`.nrm` files). ' +
      'In the grammar blocks, lower-case words such as `exp`, `name` and `ty` are placeholders ' +
      '(the site shows them in italics); everything else is literal.',
    ...body.slice(1),
  ].join('\n\n') + '\n';
}

/* ------------------------------------------------------------ search index */

/** Elements whose text is separated from what follows. */
const BLOCK = new Set(['p', 'li', 'td', 'th', 'dt', 'dd', 'pre', 'div', 'tr', 'summary']);

/** Text that is not worth searching: chrome, and whole programs. */
const skipForSearch = (n) => isChrome(n) || hasClass(n, 'example') || hasClass(n, 'trace');

function searchEntries(html, page) {
  const doc = parse(html);
  const main = find(doc, (x) => x.tag === 'main');
  if (!main) throw new Error(`${page.file} has no <main>`);
  const entries = [];
  let current = null;
  let lastId = '';
  let h2 = '';

  const open = (node, level, title) => {
    if (node.attrs.id) lastId = node.attrs.id;
    // A heading with no id of its own (a card, say) links to the section it is in.
    const id = level === 1 ? '' : lastId;
    current = { u: page.url + (id ? `#${id}` : ''), t: title, p: page.label, l: level, x: '' };
    if (node.attrs['data-short'] && node.attrs['data-short'] !== title) current.k = node.attrs['data-short'];
    if (level === 2) h2 = title;
    if (level > 2 && h2) current.s = h2;
    entries.push(current);
  };

  const visit = (n) => {
    if (n.text !== undefined) { if (current) current.x += n.text; return; }
    if (skipForSearch(n)) return;
    const h = /^h([1-4])$/.exec(n.tag);
    if (h) {
      open(n, +h[1], squash(text({ children: n.children.filter((c) => !hasClass(c, 'version')) })));
      return;
    }
    if (n.tag === 'details' && n.attrs.id) {
      const summary = n.children.find((c) => c.tag === 'summary');
      open(n, 3, squash(text(summary ?? { text: n.attrs.id })).replace(/ · .*/, ''));
      for (const c of n.children) if (c !== summary) visit(c);
      return;
    }
    if (BLOCK.has(n.tag)) {
      for (const c of n.children) visit(c);
      if (current) current.x += ' ';
      return;
    }
    for (const c of n.children) visit(c);
  };
  visit(main);
  for (const e of entries) e.x = squash(e.x.replace(/\u00a0/g, ' '));
  return entries;
}

export function searchIndex(pages) {
  const entries = pages.flatMap(({ html, page }) => searchEntries(html, page));
  return [
    '/* Generated from the site\'s pages by scripts/docs-assets.mjs. Do not edit by hand.',
    ' *',
    ' * One entry per heading: u is the link, t the heading, k its short label,',
    ' * p the page, s the enclosing section, l the level, x the section\'s text. */',
    'export const SEARCH_INDEX = [',
    ...entries.map((e) => `  ${JSON.stringify(e)},`),
    '];',
    '',
  ].join('\n');
}

/* ------------------------------------------------------------------- main */

const read = (file) => fs.readFileSync(path.join(DOCS, file), 'utf8');

/** Every generated file's expected contents, keyed by its name in docs/. */
export function generate() {
  return {
    'munorman-reference.md': referenceMarkdown(read('docs.html')),
    'search-index.js': searchIndex(PAGES.map((page) => ({ page, html: read(page.file) }))),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  for (const [file, contents] of Object.entries(generate())) {
    fs.writeFileSync(path.join(DOCS, file), contents);
    console.log(`wrote docs/${file} (${contents.length} characters)`);
  }
}
