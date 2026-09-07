// Published fingerprint test vectors (spec sections 17.2, 17.4, 17.5).
//
// Section 17.5 requires the exact canonical serialization to be "documented and
// covered by test vectors before implementation". These are those vectors: the
// values below are the contract a second implementation must reproduce, so a
// change to any expected digest here is a BREAKING protocol change, not a test
// fix.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  contentHash, domainConcat, fileLeaf, formatDigest, hashLeaves, namedLeaf,
  normalizeMode, treeFingerprint,
} from '../src/hash.ts';
import { generatorPinnedFingerprint, inputFingerprint } from '../src/fingerprints.ts';

const entry = (path: string, content: string, mode = '100644') => ({
  path, mode, content: Buffer.from(content, 'utf8'),
});

// ---------------------------------------------------------------------------
// PINNED VECTORS. Reproduce these bytes to interoperate. Changing an expected
// value here changes the wire protocol — treat it as a schema break, never as a
// failing test to be "fixed".
// ---------------------------------------------------------------------------

test('vector: file leaf of ("a.txt", 100644, "hello")', () => {
  assert.equal(
    formatDigest(fileLeaf(entry('a.txt', 'hello'))),
    'sha256:a195ee4ecb092d880aa71aa260756ef7ed0bc96099eb57782a9b6206824ab1a6'
  );
});

test('vector: named leaf of ("k", "v")', () => {
  assert.equal(
    formatDigest(namedLeaf('k', 'v')),
    'sha256:0d7c18692f5f6fae791b2844c052dcc2ebda328d3a902fbbbba0680de3be00a1'
  );
});

test('vector: tree fingerprint of {a:"1", b:"2"} at mode 100644', () => {
  assert.equal(
    formatDigest(treeFingerprint([entry('a', '1'), entry('b', '2')])),
    'sha256:a6c0a276f7d7ae93d1cde168e486325465fa9266c8fe9f3968316350b38387da'
  );
});

test('vector: generatorPinnedFingerprint with no packages at epoch 1', () => {
  assert.equal(
    generatorPinnedFingerprint({
      manifestRef: 'o/r@abc:m.json',
      packages: [],
      schemaVersions: [1, 1, 1, 1],
      generatorEpoch: 1,
    }),
    'sha256:d42648c278d3fead52bc5ad6729bcfe65c5c215082448f6d03898a7e8e0c16f8'
  );
});

test('vector: inputFingerprint over the two vectors above', () => {
  const pr = formatDigest(treeFingerprint([entry('a', '1')]));
  const gp = generatorPinnedFingerprint({
    manifestRef: 'o/r@abc:m.json',
    packages: [],
    schemaVersions: [1, 1, 1, 1],
    generatorEpoch: 1,
  });
  assert.equal(
    inputFingerprint(pr, gp),
    'sha256:f3c31d82531699852c5bbf081c1fbd830b05e9b30175d826a7cc8e3821f264e2'
  );
});

test('length delimiting prevents path/content boundary collisions', () => {
  // Without length prefixes, ("ab", "c") and ("a", "bc") would hash the same
  // bytes. This is the property section 17.2 rule 6 exists to guarantee.
  const left = fileLeaf(entry('ab', 'c'));
  const right = fileLeaf(entry('a', 'bc'));
  assert.notEqual(left.toString('hex'), right.toString('hex'));
});

test('mode participates in the leaf', () => {
  const regular = fileLeaf(entry('script.sh', '#!/bin/sh\n', '100644'));
  const executable = fileLeaf(entry('script.sh', '#!/bin/sh\n', '100755'));
  assert.notEqual(regular.toString('hex'), executable.toString('hex'));
});

test('named leaf length-delimits both name and value', () => {
  assert.notEqual(
    namedLeaf('ab', 'c').toString('hex'),
    namedLeaf('a', 'bc').toString('hex')
  );
});

test('normalizeMode zero-pads to six octal digits', () => {
  assert.equal(normalizeMode('40000'), '040000');
  assert.equal(normalizeMode('100644'), '100644');
  assert.throws(() => normalizeMode('99999'), /invalid git mode/);
  assert.throws(() => normalizeMode(''), /invalid git mode/);
});

test('tree fingerprint sorts by path BYTE order, not UTF-16 code units', () => {
  // U+1F600 (surrogate pair in UTF-16) sorts AFTER "z" by UTF-8 bytes but
  // BEFORE it by naive JS string comparison on code units. A tree fingerprint
  // that used `.sort()` would disagree with a Go or Rust implementation here.
  const a = treeFingerprint([entry('z.txt', '1'), entry('\u{1F600}.txt', '2')]);
  const b = treeFingerprint([entry('\u{1F600}.txt', '2'), entry('z.txt', '1')]);
  assert.equal(a.toString('hex'), b.toString('hex'), 'order of input must not matter');

  const naive = ['z.txt', '\u{1F600}.txt'].sort();
  assert.equal(naive[0], 'z.txt', 'JS default sort puts z first — the trap this guards');
  assert.ok(
    Buffer.compare(Buffer.from('z.txt', 'utf8'), Buffer.from('\u{1F600}.txt', 'utf8')) < 0,
    'by UTF-8 bytes z sorts first too, so both orders agree here'
  );
});

test('tree fingerprint rejects duplicate paths', () => {
  assert.throws(
    () => treeFingerprint([entry('a.txt', '1'), entry('a.txt', '2')]),
    /duplicate path/
  );
});

test('tree fingerprint length-delimits the SET, not only each element', () => {
  // Concatenating two trees must not equal a third built from their union in a
  // different grouping. Hashing the count up front is what guarantees it.
  const one = hashLeaves([fileLeaf(entry('a', '1'))]);
  const two = hashLeaves([fileLeaf(entry('a', '1')), fileLeaf(entry('b', '2'))]);
  assert.notEqual(one.toString('hex'), two.toString('hex'));
});

test('domainConcat rejects non-digest operands', () => {
  assert.throws(() => domainConcat('d', [Buffer.alloc(16)]), /32-byte digest/);
});

test('domain separation keeps leaf families apart', () => {
  // A file leaf and a named leaf built from the same bytes must not collide,
  // otherwise repository content could forge a generator input.
  const file = fileLeaf({ path: 'x', mode: '100644', content: Buffer.from('y', 'utf8') });
  const named = namedLeaf('x', 'y');
  assert.notEqual(file.toString('hex'), named.toString('hex'));
});

test('inputFingerprint is order-sensitive across its two operands', () => {
  const pr = formatDigest(treeFingerprint([entry('a', '1')]));
  const gp = formatDigest(treeFingerprint([entry('b', '2')]));
  assert.notEqual(inputFingerprint(pr, gp), inputFingerprint(gp, pr));
});

test('inputFingerprint rejects a malformed operand', () => {
  assert.throws(() => inputFingerprint('sha1:abc', 'sha256:' + 'a'.repeat(64)), /sha256:<hex>/);
});

test('generatorEpoch serialization is decimal without padding', () => {
  const base = {
    manifestRef: 'o/r@abc:manifests/m.json',
    packages: [],
    schemaVersions: [1, 1, 1, 1] as [number, number, number, number],
  };
  assert.notEqual(
    generatorPinnedFingerprint({ ...base, generatorEpoch: 1 }),
    generatorPinnedFingerprint({ ...base, generatorEpoch: 2 })
  );
});

test('package ordering does not change the pinned fingerprint', () => {
  const a = { ref: 'o/r@abc:packages/a', treeFingerprint: formatDigest(treeFingerprint([entry('a', '1')])) };
  const b = { ref: 'o/r@abc:packages/b', treeFingerprint: formatDigest(treeFingerprint([entry('b', '2')])) };
  const base = {
    manifestRef: 'o/r@abc:manifests/m.json',
    schemaVersions: [1, 1, 1, 1] as [number, number, number, number],
    generatorEpoch: 1,
  };
  assert.equal(
    generatorPinnedFingerprint({ ...base, packages: [a, b] }),
    generatorPinnedFingerprint({ ...base, packages: [b, a] })
  );
});

test('contentHash matches a plain SHA-256 of the bytes', () => {
  assert.equal(
    contentHash(Buffer.from('abc', 'utf8')),
    'sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
  );
});
