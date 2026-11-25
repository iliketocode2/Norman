/* Publish the documentation of every earlier release beside the current one.
 *
 *   node scripts/docs-versions.mjs
 *
 * For every tag vX.Y.Z other than the version in Cargo.toml, copies the
 * site as it was at that tag into docs/v/X.Y.Z/, interpreter included, so an
 * old page's examples and playground run the interpreter they were written
 * for. Then writes the list of versions into docs/versions.js, which turns
 * the version badge into a menu and marks every old page as old.
 *
 * Everything under docs/v/ is generated: this script deletes and rewrites it.
 * Run it after tagging a release and bumping Cargo.toml to the next version,
 * then commit the result (GitHub Pages and Vercel serve docs/ with no build
 * step). It needs the tags: `git fetch --tags` first in a fresh clone.
 * `node docs/verify.mjs` checks the list is current.
 */
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const DOCS = path.join(ROOT, 'docs');
const OUT = path.join(DOCS, 'v');

// Files that configure the live site, or check it, rather than being part of it.
const SKIP = new Set(['verify.mjs', 'vercel.json', 'robots.txt', 'sitemap.xml', '.nojekyll']);

const git = (...args) => execFileSync('git', args, { cwd: ROOT, maxBuffer: 1 << 28 });
const cmp = (a, b) => {
  const [x, y] = [a, b].map((v) => v.split('.').map(Number));
  return x[0] - y[0] || x[1] - y[1] || x[2] - y[2];
};

const latest = /^version\s*=\s*"([^"]+)"/m.exec(fs.readFileSync(path.join(ROOT, 'Cargo.toml'), 'utf8'))[1];
const older = git('tag', '--list', 'v*').toString().split(/\r?\n/)
  .map((t) => /^v(\d+\.\d+\.\d+)$/.exec(t.trim())?.[1])
  .filter((v) => v && v !== latest && cmp(v, latest) < 0)
  .sort(cmp).reverse();

/** A page that only sends the reader somewhere else. */
const isStub = (html) => /<meta http-equiv="refresh"/i.test(html);

/** The real pages in a directory of the site, in a stable order. */
function pagesIn(dir) {
  return fs.readdirSync(dir).filter((f) => f.endsWith('.html'))
    .filter((f) => !isStub(fs.readFileSync(path.join(dir, f), 'utf8')))
    .sort();
}

/** Mark an old page as old: unindexed, its badge showing its own version, and the switcher loaded. */
function freeze(html, version) {
  html = html.replace(/\r\n?/g, '\n');
  if (!/<meta name="robots"/i.test(html)) {
    html = html.replace(/(<meta charset="[^"]*">)/i, '$1\n<meta name="robots" content="noindex">');
  }
  // v0.3.0 shipped with its badge still reading v0.2.0.
  html = html.replace(
    /(class="version" href="[^"]*\/releases\/tag\/)v[\d.]+(" title=")[^"]*(">)v[\d.]+</,
    `$1v${version}$2This release$3v${version}<`,
  );
  // A link to index.html becomes ./, which every host serves without a redirect.
  html = html.replace(/href="index\.html(#[^"]*)?"/g, (_, hash = '') => `href="./${hash}"`);
  if (!isStub(html)) {
    html = html.replace(/<\/body>/i, `<script src="../../versions.js" data-version="${version}"></script>\n</body>`);
  }
  return html;
}

fs.rmSync(OUT, { recursive: true, force: true });
const versions = [{ version: latest, pages: pagesIn(DOCS) }];

for (const version of older) {
  const tag = `v${version}`;
  const dir = path.join(OUT, version);
  const files = git('ls-tree', '-r', '--name-only', tag, 'docs/').toString().split('\n').filter(Boolean);
  for (const file of files) {
    const rel = file.slice('docs/'.length);
    if (SKIP.has(rel) || rel.includes('/')) continue;
    let bytes = git('show', `${tag}:${file}`);
    if (rel.endsWith('.html')) bytes = freeze(bytes.toString('utf8'), version);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, rel), bytes);
  }
  versions.push({ version, pages: pagesIn(dir) });
  console.log(`wrote docs/v/${version}/ (${fs.readdirSync(dir).length} files)`);
}

const js = path.join(DOCS, 'versions.js');
const src = fs.readFileSync(js, 'utf8').replace(/\r\n?/g, '\n');
const LIST = /^( *)const VERSIONS = \[\n[\s\S]*?\n *\];/m;
const indent = LIST.exec(src)?.[1];
if (indent === undefined) throw new Error('docs/versions.js has no VERSIONS list to replace');
const list = [`${indent}const VERSIONS = [`, ...versions.map((v) => `${indent}  ${JSON.stringify(v)},`), `${indent}];`].join('\n');
const next = src.replace(LIST, list);
fs.writeFileSync(js, next);
console.log(`wrote docs/versions.js (latest ${latest}; ${older.length} older)`);
