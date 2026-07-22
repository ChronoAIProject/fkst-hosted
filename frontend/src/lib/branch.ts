/** Conservative branch-name charset shared with the control plane. */
export const BRANCH_NAME_RE = /^[A-Za-z0-9._/-]+$/;

export type BranchNameError =
  | 'empty'
  | 'too_long'
  | 'at_sign'
  | 'forbidden_start'
  | 'forbidden_end'
  | 'lock_suffix'
  | 'double_dot'
  | 'double_slash'
  | 'reflog_syntax'
  | 'invalid_character';

/** Return the first violated backend branch rule, or null when valid. */
export function validateBranchName(name: string): BranchNameError | null {
  if (name.length === 0) return 'empty';
  if (name.length > 200) return 'too_long';
  if (name === '@') return 'at_sign';
  if (name.startsWith('-') || name.startsWith('/') || name.startsWith('.')) {
    return 'forbidden_start';
  }
  if (name.endsWith('/') || name.endsWith('.')) return 'forbidden_end';
  if (name.endsWith('.lock')) return 'lock_suffix';
  if (name.includes('..')) return 'double_dot';
  if (name.includes('//')) return 'double_slash';
  if (name.includes('@{')) return 'reflog_syntax';
  if (!BRANCH_NAME_RE.test(name)) return 'invalid_character';
  return null;
}

/** Blank optional input is omitted; a populated input must pass every rule. */
export function validateOptionalBranchName(raw: string): BranchNameError | null {
  const name = raw.trim();
  return name === '' ? null : validateBranchName(name);
}
