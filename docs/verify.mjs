/* Extract every example from the site's HTML and run it through the wasm
 * interpreter, so a broken example on the page fails the build rather than
 * the reader. Run: node docs/verify.mjs */
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

let failures = 0, checked = 0;

// Every starter program in the playground.
const { EXAMPLES } = await import(new URL('./examples.js', import.meta.url));
for (const [name, source] of Object.entries(EXAMPLES)) {
  const res = await run(source);
  checked += 1;
  if (!res.ok || res.total === 0) {
    failures += 1;
    console.error(`FAIL  example: ${name}
${res.transcript}`);
  } else {
    console.log(`ok    example: ${name}  (${res.total} tests)`);
  }
}

// Every code block shown on a page.
for (const file of ['index.html', 'playground.html']) {
  const full = path.join(here, file);
  if (!fs.existsSync(full)) continue;
  const html = fs.readFileSync(full, 'utf8');
  for (const m of html.matchAll(/<pre><code>([\s\S]*?)<\/code><\/pre>/g)) {
    const source = unescape(m[1]);
    const title = (html.slice(0, m.index).match(/class="title">([^<]+)/g) || []).pop() || file;
    const name = title.replace(/.*">/, '');
    const res = await run(source);
    checked += 1;
    if (!res.ok || res.total === 0) {
      failures += 1;
      console.error(`FAIL  ${name}\n${res.transcript}`);
    } else {
      console.log(`ok    ${name}  (${res.total} tests)`);
    }
  }
}
console.log(`\n${checked - failures} of ${checked} examples pass.`);
process.exit(failures ? 1 : 0);
