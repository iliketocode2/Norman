// Tests of the harness itself, with no API calls: a mock client stands in
// for the model and writes the programs. Run with
//
//   node eval/test.mjs [--norman CMD]
//
// It checks the repair loop, the error categories, the spend reservation and
// the conversation the model would see.

import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { classify, extract, loadTasks, score, Spend, summarize, systemPrompt, taskPrompt, trial } from './run.mjs';

const i = process.argv.indexOf('--norman');
let norman = i > 0 ? process.argv[i + 1] : process.env.NORMAN;
if (!norman) {
  console.error('usage: node eval/test.mjs --norman CMD');
  process.exit(2);
}
// as run.mjs does: norman runs from inside each work directory
{
  const [exe, ...rest] = norman.split(' ').filter(Boolean);
  if (/[\\/]/.test(exe) && fs.existsSync(path.resolve(exe))) norman = [path.resolve(exe), ...rest].join(' ');
}
const opts = { norman, model: 'claude-sonnet-5-5', rounds: 3, maxTokens: 32000, effort: 'medium' };
const out = fs.mkdtempSync(path.join(os.tmpdir(), 'munorman-eval-test-'));
const [triage] = loadTasks('B01-triage');
const [words] = loadTasks('A01-word-freq');
let failures = 0;

async function test(name, fn) {
  try {
    await fn();
    console.log(`ok    ${name}`);
  } catch (e) {
    failures++;
    console.log(`FAIL  ${name}\n      ${e.message.split('\n').join('\n      ')}`);
  }
}

// A client whose replies are scripted, recording every request it gets.
function mockClient(replies) {
  const requests = [];
  return {
    requests,
    messages: {
      stream(req) {
        requests.push(structuredClone(req));
        const text = replies.shift();
        return {
          finalMessage: async () => ({
            content: [{ type: 'thinking', thinking: '', signature: 'x' }, { type: 'text', text }],
            stop_reason: 'end_turn',
            usage: { input_tokens: 100, output_tokens: 1000, cache_read_input_tokens: 8000, cache_creation_input_tokens: 0 },
          }),
        };
      },
    },
  };
}

const fence = (src) => `Here it is:\n\n\`\`\`nrm\n${src}\n\`\`\``;
const reference = fs.readFileSync(path.join(triage.dir, 'reference.nrm'), 'utf8');

await test('extract takes the last code block', () => {
  assert.equal(extract('```\na\n```\nthen\n```nrm\nb\n```'), 'b\n');
  assert.equal(extract('no code here'), null);
});

await test('the task prompt carries the interface, and the sites are parsed', () => {
  assert.match(taskPrompt(triage), /Already defined for you[\s\S]*\(datatype Triage/);
  assert.deepEqual(triage.sites, ['triage']);
  assert.deepEqual(words.sites, []);
});

await test('the system prompt holds the reference, and examples only when asked', () => {
  assert.match(systemPrompt('ref'), /# µNorman language reference/);
  assert.doesNotMatch(systemPrompt('ref'), /## cp-agent\.nrm/);
  assert.match(systemPrompt('ref+examples'), /## cp-agent\.nrm/);
});

const cases = [
  ['syntax', '(define triage (model ticket)', 'syntax'],
  ['unbound name', '(define triage (model ticket) (map (lambda (x) x) (list ticket)))', 'unbound name', 'map'],
  // 0.4.0's checker accepts this, so the harness has to catch it
  ['redefinition of a datatype', '(datatype Triage [Spam])\n(define triage (model ticket) Spam)', 'redefinition', 'Triage'],
  ['redefinition of a grant', `(grant claude (model))\n${reference}`, 'redefinition', 'claude'],
  ['redefinition of a basis name', `(define length (xs) 0)\n${reference}`, 'redefinition'],
  ['type mismatch', '(define triage (model ticket) (+ 1 "a"))', 'type mismatch'],
  ['wrong site', "(define triage (model ticket) (ask model Triage (list (Message [role User] [content ticket]))))", 'wrong site'],
  ['hidden tests fail', "(define triage (model ticket) (ask model Triage (list (Message [role User] [content ticket])) 'triage))", 'hidden tests fail'],
  ['own tests fail', `${reference}\n(check-expect (+ 1 1) 3)`, 'own tests fail'],
  ['pass', reference, 'pass'],
];
for (const [name, program, category, detail] of cases) {
  await test(`classifies: ${name}`, async () => {
    const s = await score(opts, triage, path.join(out, 'classify', name.replace(/ /g, '-')), program);
    const c = classify(triage, s, program);
    assert.equal(c.category, category, `got ${c.category}; check output:\n${s.checkOut}\ngrade output:\n${s.gradeOut}`);
    if (detail) assert.equal(c.detail, detail);
  });
}

await test('the repair loop feeds the checker output back, and stops at a clean check', async () => {
  const client = mockClient([
    'I forgot the code.',
    fence('(define triage (model ticket) (map car (list ticket)))'),
    fence(reference),
  ]);
  const t = await trial(opts, client, 'SYSTEM', triage, 1, new Spend(10), out);
  assert.deepEqual(t.rounds.map((r) => r.category), ['no code', 'unbound name', 'pass'],
    JSON.stringify(t.rounds.map((r) => ({ category: r.category, detail: r.detail, checkOut: r.checkOut }))));
  assert.equal(client.requests.length, 3);
  const last = client.requests[2].messages;
  // user task, assistant (no code), user nudge, assistant (bad), user errors
  assert.deepEqual(last.map((m) => m.role), ['user', 'assistant', 'user', 'assistant', 'user']);
  assert.match(last[2].content, /no code block/);
  assert.match(last[4].content, /unbound name map[\s\S]*Fix the program/);
  // thinking blocks go back unchanged
  assert.equal(last[1].content[0].type, 'thinking');
  // the reference is cached, and thinking and effort are set
  assert.deepEqual(client.requests[0].system[0].cache_control, { type: 'ephemeral' });
  assert.deepEqual(client.requests[0].thinking, { type: 'adaptive' });
  assert.deepEqual(client.requests[0].output_config, { effort: 'medium' });
  assert.ok(t.usd > 0);
});

await test('a trial stops after the last repair round', async () => {
  const bad = fence('(define triage (model ticket) (map car (list ticket)))');
  const client = mockClient([bad, bad, bad, bad, bad]);
  const t = await trial(opts, client, 'SYSTEM', triage, 2, new Spend(10), out);
  assert.equal(t.rounds.length, 4);
  assert.equal(client.requests.length, 4);
});

await test('a call that could exceed the cap is never made', async () => {
  const client = mockClient([fence(reference)]);
  // the worst case of one call is 32000 output tokens at $10/M = $0.32
  const t = await trial(opts, client, 'SYSTEM', triage, 3, new Spend(0.2), out);
  assert.equal(t.status, 'over budget');
  assert.equal(client.requests.length, 0);
});

await test('the summary has the tables design/14 §4 asks for', async () => {
  const client = mockClient([fence('(define triage (model ticket) (map car (list ticket)))'), fence(reference)]);
  const t = await trial(opts, client, 'SYSTEM', triage, 4, new Spend(10), out);
  const md = summarize({
    who: 'mock', condition: 'ref', stamp: 'now', commit: 'abc', dirty: false, normanVersion: 'norman 0.4.0',
    referenceHash: '0'.repeat(64), trials: 1, rounds: 3, effort: 'medium', spent: 0.01, maxSpend: 10,
    refLines: { 'B01-triage': triage.refLines },
  }, [t]);
  assert.match(md, /\| \*\*all\*\* \| 1 \| 0% \| 0% \| 100% \| 1\.00 \|/);
  assert.match(md, /`map` \(1\)/);
});

fs.rmSync(out, { recursive: true, force: true });
console.log(failures ? `\n${failures} failed` : '\nall passed');
process.exit(failures ? 1 : 0);
