import { describe, it, expect } from 'vitest';
import { ACCOUNT_NODE, columnsFor, gridPositions } from './layout';
import { levelKey, parentLevel } from './level';

describe('columnsFor', () => {
  it('grows with the square root, biased wide, capped at 5', () => {
    expect(columnsFor(0)).toBe(1);
    expect(columnsFor(1)).toBe(2); // ceil(sqrt(1.4)) = 2
    expect(columnsFor(2)).toBe(2);
    expect(columnsFor(4)).toBe(3);
    expect(columnsFor(9)).toBe(4);
    expect(columnsFor(100)).toBe(5); // cap
  });
});

describe('gridPositions', () => {
  it('lays nodes out row-major with the geometry pitch', () => {
    const geo = { width: 100, height: 50, gapX: 10, gapY: 5 };
    const positions = gridPositions(5, geo, 2);
    expect(positions).toEqual([
      { x: 0, y: 0 },
      { x: 110, y: 0 },
      { x: 0, y: 55 },
      { x: 110, y: 55 },
      { x: 0, y: 110 },
    ]);
  });

  it('returns no positions for zero nodes', () => {
    expect(gridPositions(0, ACCOUNT_NODE)).toEqual([]);
  });
});

describe('level model', () => {
  it('derives stable keys per level', () => {
    expect(levelKey({ kind: 'root' })).toBe('root');
    expect(levelKey({ kind: 'account', login: 'acme' })).toBe('account:acme');
    expect(levelKey({ kind: 'repo', owner: 'acme', name: 'widgets' })).toBe('repo:acme/widgets');
  });

  it('walks up one level at a time', () => {
    expect(parentLevel({ kind: 'repo', owner: 'acme', name: 'widgets' })).toEqual({
      kind: 'account',
      login: 'acme',
    });
    expect(parentLevel({ kind: 'account', login: 'acme' })).toEqual({ kind: 'root' });
    expect(parentLevel({ kind: 'root' })).toBeNull();
  });
});
