// Duplicate-key rejection (spec section 17.6).

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { assertNoDuplicateKeys, DuplicateKeyError, parseStrictJson } from '../src/jsonstrict.ts';

test('accepts an ordinary document', () => {
  assertNoDuplicateKeys('{"a":1,"b":{"c":[1,2,{"d":3}]}}');
});

test('rejects a duplicate key at the top level', () => {
  assert.throws(
    () => assertNoDuplicateKeys('{"outputFingerprint":"real","outputFingerprint":"forged"}'),
    DuplicateKeyError
  );
});

test('rejects a duplicate key in a nested object', () => {
  assert.throws(() => assertNoDuplicateKeys('{"source":{"head":"a","head":"b"}}'), DuplicateKeyError);
});

test('the same key in sibling objects is not a duplicate', () => {
  assertNoDuplicateKeys('{"a":{"id":1},"b":{"id":2}}');
});

test('the same key across array elements is not a duplicate', () => {
  assertNoDuplicateKeys('[{"id":1},{"id":2},{"id":3}]');
});

test('braces and colons inside strings are inert', () => {
  // A naive scanner that did not lex strings would see `{"x"` as a new object
  // and mis-track depth from here on.
  assertNoDuplicateKeys('{"note":"{\\"x\\": 1} : , {","note2":"}"}');
});

test('escaped quotes do not terminate a key', () => {
  assertNoDuplicateKeys('{"a\\"b":1,"a":2}');
  assert.throws(() => assertNoDuplicateKeys('{"a\\"b":1,"a\\"b":2}'), DuplicateKeyError);
});

test('unicode escapes are decoded before comparison', () => {
  // "a" and "a" denote the SAME key, so this must be caught. Comparing raw
  // source text would miss it.
  assert.throws(() => assertNoDuplicateKeys('{"\\u0061":1,"a":2}'), DuplicateKeyError);
});

test('array values do not confuse key tracking', () => {
  assertNoDuplicateKeys('{"list":["a","b"],"a":1}');
});

test('parseStrictJson returns the parsed value when clean', () => {
  assert.deepEqual(parseStrictJson('{"a":[1,2]}'), { a: [1, 2] });
});

test('parseStrictJson refuses a duplicated key rather than last-wins', () => {
  // JSON.parse alone would silently return {"a":2}. This is the whole point.
  assert.equal((JSON.parse('{"a":1,"a":2}') as { a: number }).a, 2);
  assert.throws(() => parseStrictJson('{"a":1,"a":2}'), DuplicateKeyError);
});

test('unterminated string is reported, not silently accepted', () => {
  assert.throws(() => assertNoDuplicateKeys('{"a":"oops'), DuplicateKeyError);
});
