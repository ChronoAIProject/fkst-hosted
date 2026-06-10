import { test } from 'node:test';
import assert from 'node:assert/strict';
import { setPending, freeze, stripLeadingComment, BEGIN, END } from './changelog.mjs';

const base = `# Changelog

Intro line.

${BEGIN}
${END}

_No releases yet._
`;

const pendingOf = (doc) => doc.slice(doc.indexOf(BEGIN) + BEGIN.length, doc.indexOf(END)).trim();

test('setPending inserts the version section inside the pending block', () => {
  const out = setPending(base, '0.1.0', '2026-06-10', '## Fixed\n- a bug');
  assert.match(out, /## v0\.1\.0 — 2026-06-10/);
  assert.ok(out.indexOf(BEGIN) < out.indexOf('## v0.1.0'));
  assert.ok(out.indexOf('## v0.1.0') < out.indexOf(END));
  assert.match(pendingOf(out), /## Fixed/);
});

test('setPending is idempotent (running twice yields the same document)', () => {
  const once = setPending(base, '0.1.0', '2026-06-10', '## Fixed\n- a bug');
  const twice = setPending(once, '0.1.0', '2026-06-10', '## Fixed\n- a bug');
  assert.equal(twice, once);
});

test('setPending strips a leading HTML comment from the note', () => {
  const note = '<!-- do not edit this template -->\n## Fixed\n- thing\n';
  const out = setPending(base, '0.2.0', '2026-06-11', note);
  assert.ok(!out.includes('do not edit'));
  assert.match(out, /## Fixed/);
});

test('freeze moves the pending entry below END and empties pending', () => {
  const pending = setPending(base, '0.1.0', '2026-06-10', '## Fixed\n- a bug');
  const frozen = freeze(pending);
  assert.equal(pendingOf(frozen), '');
  assert.ok(frozen.indexOf('## v0.1.0') > frozen.indexOf(END));
});

test('freeze keeps older releases below the newer one after a second cycle', () => {
  const r1 = freeze(setPending(base, '0.1.0', '2026-06-10', '## Fixed\n- one'));
  const r2 = freeze(setPending(r1, '0.2.0', '2026-06-12', '## New Feature\n- two'));
  assert.ok(r2.indexOf('## v0.2.0') < r2.indexOf('## v0.1.0'), 'newest on top');
  assert.equal(pendingOf(r2), '');
});

test('freeze is a no-op when nothing is pending', () => {
  const out = freeze(base);
  assert.equal(pendingOf(out), '');
  assert.ok(!/## v/.test(out));
});

test('stripLeadingComment only strips a leading comment', () => {
  assert.equal(stripLeadingComment('<!-- x -->\nbody'), 'body');
  assert.equal(stripLeadingComment('body <!-- x -->'), 'body <!-- x -->');
});
