import { describe, expect, it } from 'vitest';
import { levelToParams, paramsToLevel } from './level';
import type { CanvasLevel } from './level';

/** The mapping's query string, for readable assertions. */
const query = (level: CanvasLevel, selectedKey?: string | null) =>
  levelToParams(level, selectedKey).toString();

const parse = (search: string) => paramsToLevel(new URLSearchParams(search));

describe('levelToParams', () => {
  it('maps the root level to no parameters at all', () => {
    // `/dashboard` stays the canonical clean URL for the root view.
    expect(query({ kind: 'root' })).toBe('');
  });

  it('maps an account level to owner', () => {
    expect(query({ kind: 'account', login: 'acme' })).toBe('owner=acme');
  });

  it('maps a repo level to owner and repo', () => {
    expect(query({ kind: 'repo', owner: 'acme', name: 'site' })).toBe('owner=acme&repo=site');
  });

  it('adds the session only alongside a repo', () => {
    expect(query({ kind: 'repo', owner: 'acme', name: 'site' }, 'sess-1')).toBe(
      'owner=acme&repo=site&session=sess-1'
    );
    // A session means nothing without a repo to select it in.
    expect(query({ kind: 'account', login: 'acme' }, 'sess-1')).toBe('owner=acme');
    expect(query({ kind: 'root' }, 'sess-1')).toBe('');
  });

  it('omits an empty or absent session key', () => {
    const level: CanvasLevel = { kind: 'repo', owner: 'acme', name: 'site' };
    expect(query(level, null)).toBe('owner=acme&repo=site');
    expect(query(level, '')).toBe('owner=acme&repo=site');
  });

  it('encodes values that need it', () => {
    expect(query({ kind: 'repo', owner: 'a c', name: 'b&d' }, 'trigger-7')).toBe(
      'owner=a+c&repo=b%26d&session=trigger-7'
    );
  });
});

describe('paramsToLevel', () => {
  it('round-trips every level', () => {
    const levels: CanvasLevel[] = [
      { kind: 'root' },
      { kind: 'account', login: 'acme' },
      { kind: 'repo', owner: 'acme', name: 'site' },
    ];
    for (const level of levels) {
      expect(paramsToLevel(levelToParams(level)).level).toEqual(level);
    }
  });

  it('round-trips a repo level with its session', () => {
    const level: CanvasLevel = { kind: 'repo', owner: 'acme', name: 'site' };
    expect(paramsToLevel(levelToParams(level, 'sess-1'))).toEqual({
      level,
      sessionKey: 'sess-1',
    });
  });

  it('falls back to root with no parameters', () => {
    expect(parse('')).toEqual({ level: { kind: 'root' } });
  });

  it('ignores a repo without an owner', () => {
    // There is no repository without an account, so a truncated URL degrades to
    // the nearest sensible level rather than rendering something broken.
    expect(parse('repo=site')).toEqual({ level: { kind: 'root' } });
    expect(parse('repo=site&session=sess-1')).toEqual({ level: { kind: 'root' } });
  });

  it('ignores a session without a repo', () => {
    expect(parse('owner=acme&session=sess-1')).toEqual({
      level: { kind: 'account', login: 'acme' },
    });
  });

  it('treats blank values as absent', () => {
    expect(parse('owner=%20%20')).toEqual({ level: { kind: 'root' } });
    expect(parse('owner=acme&repo=%20')).toEqual({ level: { kind: 'account', login: 'acme' } });
    expect(parse('owner=acme&repo=site&session=%20')).toEqual({
      level: { kind: 'repo', owner: 'acme', name: 'site' },
    });
  });

  it('trims surrounding whitespace', () => {
    expect(parse('owner=%20acme%20&repo=%20site%20')).toEqual({
      level: { kind: 'repo', owner: 'acme', name: 'site' },
    });
  });

  it('ignores unrelated parameters', () => {
    expect(parse('owner=acme&utm_source=chat')).toEqual({
      level: { kind: 'account', login: 'acme' },
    });
  });
});
