import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { LanguageProvider, useContent } from './index';
import { en } from './en';
import { zh } from './zh';
import { LanguageToggle } from '@/components/layout/language-toggle';
import { Rich } from '@/components/content/rich';

function Probe() {
  const c = useContent();
  return <div>{c.nav.home}</div>;
}

// Collect every leaf key path of a catalog. Objects recurse by key; arrays
// recurse by index (so array length and per-element object shape are compared,
// not just presence). Used to prove the per-domain split kept `en` and `zh`
// structurally identical.
function keyPaths(value: unknown, prefix = ''): string[] {
  if (Array.isArray(value)) {
    return value.flatMap((item, i) => keyPaths(item, `${prefix}[${i}]`));
  }
  if (value !== null && typeof value === 'object') {
    return Object.entries(value).flatMap(([k, v]) =>
      keyPaths(v, prefix ? `${prefix}.${k}` : k)
    );
  }
  return [prefix];
}

describe('i18n', () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it('defaults to English and switches to 简体中文 via the toggle', async () => {
    const user = userEvent.setup();
    render(
      <LanguageProvider>
        <LanguageToggle />
        <Probe />
      </LanguageProvider>
    );

    expect(screen.getByText('Home')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '中文' }));

    expect(screen.getByText('首页')).toBeInTheDocument();
    expect(document.documentElement.lang).toBe('zh');
    expect(window.localStorage.getItem('fkst-lang')).toBe('zh');
  });

  it('useContent works without a provider (English default)', () => {
    render(<Probe />);
    expect(screen.getByText('Home')).toBeInTheDocument();
  });
});

describe('i18n key parity (per-domain split)', () => {
  it('en and zh expose identical key structure — every key exists in both', () => {
    const enPaths = keyPaths(en).sort();
    const zhPaths = keyPaths(zh).sort();
    // toEqual on sorted arrays proves both directions at once: any key present
    // in one language but not the other makes the arrays differ.
    expect(zhPaths).toEqual(enPaths);
  });

  it('surfaces a missing key rather than passing silently', () => {
    // Guards the guard: keyPaths must actually distinguish differing shapes,
    // otherwise the parity assertion above could never fail.
    expect(keyPaths({ a: 1, b: { c: 2 } }).sort()).toEqual(['a', 'b.c']);
    expect(keyPaths({ a: ['x', 'y'] })).toEqual(['a[0]', 'a[1]']);
    expect(keyPaths({ a: 1 })).not.toEqual(keyPaths({ a: 1, b: 2 }));
  });
});

describe('Rich', () => {
  it('renders `code` as <code> and **bold** as emphasis', () => {
    render(<Rich>{'run `gh` with **care**'}</Rich>);
    const code = screen.getByText('gh');
    expect(code.tagName).toBe('CODE');
    expect(screen.getByText('care')).toBeInTheDocument();
  });
});
