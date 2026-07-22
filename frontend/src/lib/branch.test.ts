import { describe, expect, it } from 'vitest';
import { BRANCH_NAME_RE, validateBranchName, validateOptionalBranchName } from './branch';

describe('branch-name validation', () => {
  it.each(['main', 'release/v1.2', 'feature_one', 'fkst-hosted-default'])(
    'accepts %s',
    (name) => {
      expect(validateBranchName(name)).toBeNull();
      expect(BRANCH_NAME_RE.test(name)).toBe(true);
    }
  );

  it.each([
    ['', 'empty'],
    ['a'.repeat(201), 'too_long'],
    ['@', 'at_sign'],
    ['-topic', 'forbidden_start'],
    ['/topic', 'forbidden_start'],
    ['.topic', 'forbidden_start'],
    ['topic/', 'forbidden_end'],
    ['topic.', 'forbidden_end'],
    ['topic.lock', 'lock_suffix'],
    ['topic..next', 'double_dot'],
    ['topic//next', 'double_slash'],
    ['topic@{next', 'reflog_syntax'],
    ['topic next', 'invalid_character'],
    ['topic~next', 'invalid_character'],
  ] as const)('rejects %s with %s', (name, reason) => {
    expect(validateBranchName(name)).toBe(reason);
  });

  it('treats blank optional input as omitted and validates its trimmed value', () => {
    expect(validateOptionalBranchName('   ')).toBeNull();
    expect(validateOptionalBranchName(' release/v2 ')).toBeNull();
    expect(validateOptionalBranchName(' bad branch ')).toBe('invalid_character');
  });
});
