#!/usr/bin/env node
// The authoring eval's harness (design/14, Stage 1).
//
// For each task and trial: give a model the µNorman reference and the task,
// take the program it writes, check it with `norman` and grade it against
// the task's hidden tests, and feed the checker's output back for up to
// --rounds repairs. Everything a run does is written under --out; `summary.md`
// there holds the tables design/14 §4 asks for.
//
//   node eval/run.mjs --model claude-sonnet-5-5 [options]
//   node eval/run.mjs --replay reference|naive [options]   (no API: plumbing test)
//   node eval/run.mjs --summarize eval/runs/<dir>          (rebuild summary.md)
//
// Spending is capped the way µNorman caps it: before each call, its worst
// case (the whole input at the cache-write price, plus max_tokens of output)
// is reserved against --max-spend, and a call that doesn't fit isn't made.

import Anthropic from '@anthropic-ai/sdk';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.join(here, '..');

// ---------------------------------------------------------------- options

const USAGE = `usage: node eval/run.mjs (--model ID | --replay reference|naive) [options]
       node eval/run.mjs --summarize DIR

  --model ID          the model that writes the programs
  --replay WHICH      instead of a model, answer with each task's reference.nrm
                      or naive.nrm (free; tests the harness itself)
  --condition C       ref (default) or ref+examples
  --trials N          trials per task (default 3)
  --rounds N          repair rounds after the first program (default 3)
  --tasks GLOB        which tasks, e.g. 'B*' (default: all)
  --effort E          low|medium|high|xhigh|max (default medium)
  --max-tokens N      per call (default 32000)
  --max-spend D       dollars for the whole run (default 10)
  --concurrency N     trials in flight at once (default 4)
  --norman CMD        how to run norman, e.g. 'target/release/norman' or
                      'wsl -e /home/me/target/debug/norman' (default: $NORMAN,
                      else target/release/norman)
  --out DIR           where to write the run (default eval/runs/<stamp>-<model>-<condition>)
  --summarize DIR     rebuild DIR/summary.md from DIR/trials.jsonl, and exit`;

function parseArgs(argv) {
  const o = {
    condition: 'ref', trials: 3, rounds: 3, tasks: '*', effort: 'medium',
    maxTokens: 32000, maxSpend: 10, concurrency: 4,
    norman: process.env.NORMAN || path.join(repo, 'target', 'release', 'norman'),
  };
  const need = (i) => {
    if (i + 1 >= argv.length) die(`${argv[i]} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--model': o.model = need(i++); break;
      case '--replay': o.replay = need(i++); break;
      case '--condition': o.condition = need(i++); break;
      case '--trials': o.trials = +need(i++); break;
      case '--rounds': o.rounds = +need(i++); break;
      case '--tasks': o.tasks = need(i++); break;
      case '--effort': o.effort = need(i++); break;
      case '--max-tokens': o.maxTokens = +need(i++); break;
      case '--max-spend': o.maxSpend = +need(i++); break;
      case '--concurrency': o.concurrency = +need(i++); break;
      case '--norman': o.norman = need(i++); break;
      case '--out': o.out = need(i++); break;
      case '--summarize': o.summarize = need(i++); break;
      case '-h': case '--help': console.log(USAGE); process.exit(0);
      default: die(`unknown argument ${a}`);
    }
  }
  // norman runs from inside each work directory, so a relative path to it
  // must be resolved here, against the directory the harness started in.
  const [exe, ...rest] = o.norman.split(' ').filter(Boolean);
  if (/[\\/]/.test(exe) && fs.existsSync(path.resolve(exe))) o.norman = [path.resolve(exe), ...rest].join(' ');
  if (o.summarize) return o;
  if (!o.model === !o.replay) die('give exactly one of --model and --replay');
  if (o.replay && !['reference', 'naive'].includes(o.replay)) die('--replay is reference or naive');
  if (!['ref', 'ref+examples'].includes(o.condition)) die('--condition is ref or ref+examples');
  if (o.model && !PRICES[o.model]) die(`no prices known for ${o.model}; add them to PRICES in eval/run.mjs`);
  return o;
}

function die(msg) {
  console.error(`${msg}\n\n${USAGE}`);
  process.exit(2);
}

// ---------------------------------------------------------------- prices

// Dollars per million tokens. A five-minute cache write costs 1.25 × input.
// From the Anthropic price list as of 2026-11; Haiku 5.5's cache-read price
// is assumed to be a tenth of input. These estimate spend and enforce
// --max-spend; the bill itself is in the Console.
const PRICES = {
  'claude-sonnet-5-5': { in: 2, out: 10, read: 0.1 },
  'claude-haiku-5-5': { in: 0.1, out: 0.5, read: 0.01 },
  'claude-opus-5-5': { in: 4, out: 20, read: 0.2 },
};
const WRITE = 1.25;

function cost(model, u) {
  const p = PRICES[model];
  return ((u.input_tokens || 0) * p.in
    + (u.cache_creation_input_tokens || 0) * p.in * WRITE
    + (u.cache_read_input_tokens || 0) * p.read
    + (u.output_tokens || 0) * p.out) / 1e6;
}

// ---------------------------------------------------------------- prompts

const PREAMBLE = `You write programs in µNorman, a small programming language for AI agents. Its complete reference is below. Use only what the reference describes.

When you are given a task, reply with the complete program in a single fenced code block marked \`\`\`nrm. The program is loaded after the definitions the task says are already defined, so it may use them, and must not define them again. It may include its own unit tests. If you are shown errors from running your program, reply with the complete corrected program, again in a single code block.`;

function systemPrompt(condition) {
  let text = `${PREAMBLE}\n\n# The µNorman reference\n\n${read(path.join(repo, 'docs', 'munorman-reference.md'))}`;
  if (condition === 'ref+examples') {
    const dir = path.join(repo, 'examples');
    text += '\n\n# Example programs\n';
    for (const f of fs.readdirSync(dir).filter((f) => f.endsWith('.nrm')).sort()) {
      text += `\n## ${f}\n\n\`\`\`nrm\n${read(path.join(dir, f))}\`\`\`\n`;
    }
  }
  return text;
}

function taskPrompt(task) {
  return `${task.md.trim()}\n\nAlready defined for you (don't redefine these):\n\n\`\`\`nrm\n${task.iface}\`\`\``;
}

const REPAIR = 'Running your program printed the output above. Fix the program, and reply with the complete corrected program in a single ```nrm code block.';
const NO_CODE = 'Your reply had no code block. Reply with the complete program in a single ```nrm code block.';

// ---------------------------------------------------------------- tasks

function read(f) {
  return fs.readFileSync(f, 'utf8').replace(/\r\n/g, '\n');
}

function globToRegExp(glob) {
  return new RegExp(`^${glob.replace(/[.+^${}()|\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`);
}

function loadTasks(glob) {
  const root = path.join(here, 'tasks');
  const re = globToRegExp(glob);
  return fs.readdirSync(root).filter((n) => re.test(n)).sort().map((name) => {
    const dir = path.join(root, name);
    const md = read(path.join(dir, 'task.md'));
    return {
      name, dir, tier: name[0], md,
      iface: read(path.join(dir, 'interface.nrm')),
      tests: read(path.join(dir, 'tests.nrm')),
      // the site names task.md requires, from its **Sites:** paragraph
      sites: [...(md.split('**Sites:**')[1] || '').split('\n\n')[0].matchAll(/`'([^`]+)`/g)].map((m) => m[1]),
      refLines: lines(read(path.join(dir, 'reference.nrm'))),
    };
  });
}

// non-blank, non-comment lines
function lines(src) {
  return src.split('\n').filter((l) => l.trim() && !l.trim().startsWith(';')).length;
}

// ---------------------------------------------------------------- norman

function runOnce(cmd, cwd, file) {
  const [exe, ...pre] = cmd.split(' ').filter(Boolean);
  return new Promise((resolve) => {
    execFile(exe, [...pre, file], { cwd, timeout: 120_000, maxBuffer: 1 << 24 }, (err, stdout, stderr) => {
      const code = err ? (typeof err.code === 'number' ? err.code : -1) : 0;
      resolve({ code, out: `${stderr}${stdout}`.replace(/\r\n/g, '\n') });
    });
  });
}

// norman exits 0, 1 or 2. Any other code means the launcher failed (on
// Windows, `wsl -e` occasionally does), not the program, so try again
// rather than score a launcher fault as the model's mistake.
async function runNorman(cmd, cwd, file) {
  let r;
  for (let attempt = 0; attempt < 3; attempt++) {
    r = await runOnce(cmd, cwd, file);
    if ([0, 1, 2].includes(r.code)) return r;
  }
  return { code: r.code, out: `${r.out}\n(harness: norman could not be run; exit code ${r.code})` };
}

// The program in a reply: the last fenced code block.
function extract(text) {
  const blocks = [...text.matchAll(/```[^\n`]*\n([\s\S]*?)```/g)];
  return blocks.length ? blocks[blocks.length - 1][1] : null;
}

// Score one program: run the repair loop's check, then the hidden tests.
async function score(opts, task, dir, program) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'interface.nrm'), task.iface);
  fs.writeFileSync(path.join(dir, 'tests.nrm'), task.tests);
  fs.writeFileSync(path.join(dir, 'solution.nrm'), program);
  fs.writeFileSync(path.join(dir, 'check.nrm'), '(use interface.nrm)\n(use solution.nrm)\n');
  const check = await runNorman(opts.norman, dir, 'check.nrm');
  const grade = await runNorman(opts.norman, dir, 'tests.nrm');
  const hidden = grade.out.match(/^tests\.nrm: (.*)$/m)?.[1] ?? null;
  const own = check.out.match(/^solution\.nrm: (.*)$/m)?.[1] ?? null;
  return {
    checkClean: check.code === 0,
    checkOut: check.out,
    gradeOut: grade.out,
    hidden,
    pass: hidden !== null && /^(All \d+ tests passed|The only test passed)\.$/.test(hidden),
    selfTests: own !== null,
    selfPass: own !== null && /^(All \d+ tests passed|The only test passed)\.$/.test(own),
    category: null,
    unbound: null,
  };
}

// The names an interface defines: grants, datatypes, records.
function interfaceNames(iface) {
  return [...iface.matchAll(/^\((?:grant|datatype|record)\s+\(?([^\s()]+)/gm)].map((m) => m[1]);
}

// design/14 §3: one primary category per failing round, from the first error.
function classify(task, s, program) {
  // 0.4.0's checker rejects a second grant but silently lets a second
  // datatype or record replace the first, so the hidden tests then fail on
  // "unknown constructor". Look for it directly.
  const redefined = interfaceNames(task.iface).find((n) =>
    new RegExp(`^\\((?:grant|datatype|record)\\s+\\(?${n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[\\s)]`, 'm').test(program ?? ''));
  if (redefined && !s.pass) return { category: 'redefinition', detail: redefined };
  if (s.checkClean) {
    if (s.pass) return { category: 'pass' };
    const site = s.gradeOut.match(/no reply left for (?:ask|call|consult) site '([^']+)'/);
    if (site && !task.sites.includes(site[1])) return { category: 'wrong site', detail: site[1] };
    return { category: 'hidden tests fail' };
  }
  const first = s.checkOut.split('\n').find((l) => l.trim() && !/^The program was not run/.test(l) && !/^solution\.nrm: /.test(l)) || '';
  let m;
  if (/syntax error|unclosed|unexpected/.test(first)) return { category: 'syntax' };
  if (/is already defined|can't be redefined/.test(first)) return { category: 'redefinition' };
  if ((m = first.match(/unbound name (\S+)/))) return { category: 'unbound name', detail: m[1] };
  if (/expected .* but found/.test(first)) return { category: 'type mismatch' };
  if (/^type error:/.test(first)) return { category: 'other check error' };
  if (/Check-\w+ failed|check-\w+ failed/i.test(first)) return { category: 'own tests fail' };
  if (/run-time error/.test(first)) return { category: 'run-time error' };
  return { category: 'other', detail: first.slice(0, 120) };
}

// ---------------------------------------------------------------- the model

class Spend {
  constructor(cap) { this.cap = cap; this.spent = 0; this.reserved = 0; }
  reserve(amount) {
    if (this.spent + this.reserved + amount > this.cap) return false;
    this.reserved += amount;
    return true;
  }
  settle(reservation, actual) { this.reserved -= reservation; this.spent += actual; }
}

async function ask(opts, client, system, messages, spend) {
  if (opts.replay) return null; // answered by the caller
  const p = PRICES[opts.model];
  const chars = system.length + JSON.stringify(messages).length;
  // Worst case: every input token written to the cache (~3 chars a token is
  // generous for this text), and the whole output allowance used.
  const worst = ((chars / 3) * p.in * WRITE + opts.maxTokens * p.out) / 1e6;
  if (!spend.reserve(worst)) return { overBudget: true };
  try {
    const stream = client.messages.stream({
      model: opts.model,
      max_tokens: opts.maxTokens,
      thinking: { type: 'adaptive' },
      output_config: { effort: opts.effort },
      system: [{ type: 'text', text: system, cache_control: { type: 'ephemeral' } }],
      messages,
    });
    const msg = await stream.finalMessage();
    const usd = cost(opts.model, msg.usage);
    spend.settle(worst, usd);
    const text = msg.content.filter((b) => b.type === 'text').map((b) => b.text).join('');
    return { msg, text, usd, usage: msg.usage, stop: msg.stop_reason };
  } catch (error) {
    spend.settle(worst, 0);
    if (error instanceof Anthropic.APIError) {
      const message = `${error.status ?? ''} ${error.message}`.trim();
      // Bad credentials, a bad request or an unknown model will fail every
      // call the same way: stop the run instead of recording it N times.
      if ([400, 401, 403, 404].includes(error.status)) spend.fatal ??= message;
      return { apiError: message };
    }
    // An SDK error raised before any request was sent, such as no
    // credentials: every call would fail the same way.
    if (error instanceof Anthropic.AnthropicError) {
      spend.fatal ??= `${error.message} (set ANTHROPIC_API_KEY, or log in with \`ant auth login\`)`;
      return { apiError: error.message };
    }
    throw error;
  }
}

// One trial: up to 1 + rounds programs.
async function trial(opts, client, system, task, n, spend, outDir) {
  const t = { task: task.name, tier: task.tier, trial: n, rounds: [], usd: 0, status: 'done' };
  const messages = [{ role: 'user', content: taskPrompt(task) }];
  for (let r = 0; r <= opts.rounds; r++) {
    let text;
    let reply = null;
    if (opts.replay) {
      text = `\`\`\`nrm\n${read(path.join(task.dir, `${opts.replay}.nrm`))}\`\`\``;
    } else {
      reply = await ask(opts, client, system, messages, spend);
      if (reply.overBudget) { t.status = 'over budget'; break; }
      if (reply.apiError) { t.status = 'api error'; t.error = reply.apiError; break; }
      t.usd += reply.usd;
      text = reply.text;
    }
    const round = { round: r, stop: reply?.stop ?? 'replay', usage: reply?.usage ?? null, usd: reply?.usd ?? 0, reply: text };
    const program = extract(text);
    if (program === null) {
      Object.assign(round, { program: null, category: reply?.stop === 'refusal' ? 'refused' : 'no code', pass: false, checkClean: false });
      t.rounds.push(round);
      if (r < opts.rounds) {
        messages.push({ role: 'assistant', content: reply.msg.content });
        messages.push({ role: 'user', content: NO_CODE });
      }
      continue;
    }
    const s = await score(opts, task, path.join(outDir, 'work', task.name, `t${n}`, `r${r}`), program);
    const c = classify(task, s, program);
    Object.assign(round, {
      program, lines: lines(program), checkClean: s.checkClean, pass: s.pass, hidden: s.hidden,
      selfTests: s.selfTests, selfPass: s.selfPass, category: c.category, detail: c.detail ?? null,
      checkOut: s.checkOut, gradeOut: s.gradeOut,
    });
    t.rounds.push(round);
    // A clean check ends the trial: the program checks and its own tests pass.
    if (s.checkClean || r === opts.rounds || opts.replay) break;
    messages.push({ role: 'assistant', content: reply.msg.content });
    messages.push({ role: 'user', content: `${s.checkOut.trim() || '(no output)'}\n\n${REPAIR}` });
  }
  return t;
}

// ---------------------------------------------------------------- summary

const pct = (a, b) => (b ? `${Math.round((100 * a) / b)}%` : '–');

function summarize(meta, trials) {
  const done = trials.filter((t) => t.rounds.length > 0);
  const first = (t) => t.rounds[0];
  const last = (t) => t.rounds[t.rounds.length - 1];
  const rows = (subset) => {
    const n = subset.length;
    const check1 = subset.filter((t) => first(t).checkClean).length;
    const pass1 = subset.filter((t) => first(t).pass).length;
    const passR = subset.filter((t) => last(t).pass).length;
    const cleaned = subset.filter((t) => last(t).checkClean);
    const rounds = cleaned.length ? (cleaned.reduce((a, t) => a + t.rounds.length - 1, 0) / cleaned.length).toFixed(2) : '–';
    return { n, check1: pct(check1, n), pass1: pct(pass1, n), passR: pct(passR, n), rounds };
  };
  let md = `# Authoring eval: ${meta.who}, ${meta.condition}\n\n`;
  md += `Run ${meta.stamp}. Commit ${meta.commit}${meta.dirty ? ' (with uncommitted changes)' : ''}; ${meta.normanVersion}; reference sha256 ${meta.referenceHash.slice(0, 12)}. `;
  md += `${meta.trials} trials, up to ${meta.rounds} repair rounds, effort ${meta.effort}. Estimated spend $${meta.spent.toFixed(2)} of a $${meta.maxSpend} cap.\n\n`;
  const unfinished = trials.filter((t) => t.status !== 'done');
  if (unfinished.length) {
    md += `**${unfinished.length} of ${trials.length} trials didn't finish** (${[...new Set(unfinished.map((t) => t.status))].join(', ')}). `;
    md += 'A trial with no rounds at all is left out of the tables; one cut short counts with the rounds it had.\n\n';
  }

  md += '## Overall and by tier\n\n| | trials | check@1 | pass@1 | pass@R | mean repair rounds to a clean check |\n|---|---|---|---|---|---|\n';
  const all = rows(done);
  md += `| **all** | ${all.n} | ${all.check1} | ${all.pass1} | ${all.passR} | ${all.rounds} |\n`;
  for (const tier of [...new Set(done.map((t) => t.tier))].sort()) {
    const r = rows(done.filter((t) => t.tier === tier));
    md += `| ${tier} | ${r.n} | ${r.check1} | ${r.pass1} | ${r.passR} | ${r.rounds} |\n`;
  }

  md += '\n## By task\n\n| task | pass@1 | pass@R | reference lines | model lines (final, median) | final categories |\n|---|---|---|---|---|---|\n';
  for (const name of [...new Set(done.map((t) => t.task))].sort()) {
    const ts = done.filter((t) => t.task === name);
    const ls = ts.map((t) => last(t).lines).filter((x) => x != null).sort((a, b) => a - b);
    const cats = ts.map((t) => last(t).category).join(', ');
    md += `| ${name} | ${ts.filter((t) => first(t).pass).length}/${ts.length} | ${ts.filter((t) => last(t).pass).length}/${ts.length} | ${meta.refLines[name]} | ${ls.length ? ls[Math.floor(ls.length / 2)] : '–'} | ${cats} |\n`;
  }

  const allRounds = done.flatMap((t) => t.rounds);
  const failing = allRounds.filter((r) => r.category !== 'pass');
  const count = (xs) => Object.entries(xs.reduce((a, x) => ({ ...a, [x]: (a[x] || 0) + 1 }), {})).sort((a, b) => b[1] - a[1]);
  md += '\n## What went wrong\n\nPrimary category of every failing round (design/14 §3).\n\n| category | first rounds | all rounds |\n|---|---|---|\n';
  const firstFailing = done.map(first).filter((r) => r.category !== 'pass');
  for (const [cat, k] of count(failing.map((r) => r.category))) {
    md += `| ${cat} | ${firstFailing.filter((r) => r.category === cat).length} | ${k} |\n`;
  }
  const unbound = count(failing.filter((r) => r.category === 'unbound name').map((r) => r.detail)).slice(0, 10);
  if (unbound.length) md += `\nMost common unbound names: ${unbound.map(([n, k]) => `\`${n}\` (${k})`).join(', ')}.\n`;
  const sites = count(failing.filter((r) => r.category === 'wrong site').map((r) => r.detail)).slice(0, 5);
  if (sites.length) md += `\nWrong sites: ${sites.map(([n, k]) => `\`${n}\` (${k})`).join(', ')}.\n`;
  // "other" means the classifier didn't recognize the first error: show it,
  // so a new kind of mistake (or a harness fault) is seen, not counted.
  const others = count(failing.filter((r) => r.category === 'other').map((r) => r.detail));
  if (others.length) md += `\nUnrecognized first errors (category "other"):\n\n${others.map(([d, k]) => `- (${k}) \`${d}\``).join('\n')}\n`;

  const programs = allRounds.filter((r) => r.program);
  const withTests = programs.filter((r) => r.selfTests);
  md += `\n## Self-tests\n\n${pct(withTests.length, programs.length)} of programs had tests of their own, and ${pct(withTests.filter((r) => r.selfPass).length, withTests.length)} of those passed them.\n`;
  return md;
}

// ---------------------------------------------------------------- main

function git(args) {
  return new Promise((resolve) => execFile('git', args, { cwd: repo }, (err, out) => resolve(err ? '' : out.trim())));
}

async function pool(items, n, fn, stopped = () => false) {
  const results = [];
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, async () => {
    while (next < items.length && !stopped()) {
      const i = next++;
      results[i] = await fn(items[i], i);
    }
  }));
  return results;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.summarize) {
    const dir = opts.summarize;
    const meta = JSON.parse(read(path.join(dir, 'meta.json')));
    const trials = read(path.join(dir, 'trials.jsonl')).trim().split('\n').map((l) => JSON.parse(l));
    fs.writeFileSync(path.join(dir, 'summary.md'), summarize(meta, trials));
    console.log(`wrote ${path.join(dir, 'summary.md')}`);
    return;
  }

  const who = opts.model ?? `replay-${opts.replay}`;
  const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  const outDir = opts.out ?? path.join(here, 'runs', `${stamp}-${who}-${opts.condition}`);
  fs.mkdirSync(outDir, { recursive: true });

  const tasks = loadTasks(opts.tasks);
  if (!tasks.length) die(`no tasks match ${opts.tasks}`);
  const system = systemPrompt(opts.condition);
  const version = await runNorman(opts.norman, repo, '--version');
  if (version.code !== 0) die(`can't run norman as '${opts.norman}':\n${version.out}`);

  const meta = {
    who, model: opts.model ?? null, replay: opts.replay ?? null, condition: opts.condition, stamp,
    trials: opts.trials, rounds: opts.rounds, effort: opts.effort, maxTokens: opts.maxTokens, maxSpend: opts.maxSpend,
    commit: (await git(['rev-parse', '--short', 'HEAD'])) || 'unknown',
    dirty: (await git(['status', '--porcelain'])) !== '',
    normanVersion: version.out.trim(),
    referenceHash: createHash('sha256').update(read(path.join(repo, 'docs', 'munorman-reference.md'))).digest('hex'),
    systemHash: createHash('sha256').update(system).digest('hex'),
    refLines: Object.fromEntries(tasks.map((t) => [t.name, t.refLines])),
    spent: 0,
  };

  let client = null;
  if (!opts.replay) {
    try {
      client = new Anthropic({ maxRetries: 5 });
    } catch (error) {
      die(`can't create an API client: ${error.message}\nSet ANTHROPIC_API_KEY, or log in with \`ant auth login\`.`);
    }
  }
  const spend = new Spend(opts.maxSpend);
  const jobs = tasks.flatMap((task) => Array.from({ length: opts.trials }, (_, n) => ({ task, n })));
  const log = fs.createWriteStream(path.join(outDir, 'trials.jsonl'));
  console.log(`${who}, ${opts.condition}: ${tasks.length} tasks × ${opts.trials} trials → ${outDir}`);

  const trials = await pool(jobs, opts.replay ? 1 : opts.concurrency, async ({ task, n }) => {
    const t = await trial(opts, client, system, task, n, spend, outDir);
    log.write(`${JSON.stringify(t)}\n`);
    const r = t.rounds[t.rounds.length - 1];
    console.log(`  ${task.name} #${n}: ${t.status === 'done' ? `${r.pass ? 'PASS' : 'fail'} after ${t.rounds.length} round(s), ${r.category}` : t.status}  ($${spend.spent.toFixed(3)} spent)`);
    return t;
  }, () => spend.fatal !== undefined);
  await new Promise((resolve) => log.end(resolve));

  meta.spent = spend.spent;
  if (spend.fatal) {
    meta.stopped = spend.fatal;
    fs.writeFileSync(path.join(outDir, 'meta.json'), `${JSON.stringify(meta, null, 2)}\n`);
    console.error(`\nstopped: ${spend.fatal}\nNothing more was asked. What ran is in ${outDir}.`);
    process.exit(1);
  }
  fs.writeFileSync(path.join(outDir, 'meta.json'), `${JSON.stringify(meta, null, 2)}\n`);
  fs.writeFileSync(path.join(outDir, 'summary.md'), summarize(meta, trials));
  console.log(`\nwrote ${path.join(outDir, 'summary.md')}  (estimated spend $${spend.spent.toFixed(2)})`);
}

// Run when invoked as a script; tests import the pieces instead.
export { classify, extract, loadTasks, score, Spend, summarize, systemPrompt, taskPrompt, trial };
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
