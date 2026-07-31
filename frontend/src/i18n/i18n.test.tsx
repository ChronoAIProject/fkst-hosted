import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { LanguageProvider, useContent } from './index';
import { en } from './en';
import { zh } from './zh';
import {
  ACTOR_KINDS,
  ATTRIBUTION_SOURCES,
  DELIVERY_STATES,
  LIFECYCLE_ACTIONS,
  OPERATIONS_ERROR_CODES,
  OUTCOMES,
  PRINCIPAL_KINDS,
  RECORD_KINDS,
  ROW_KINDS,
  SANDBOX_BACKENDS,
  SANDBOX_STATUSES,
  SANDBOX_WARNINGS,
  SOURCE_HEALTHS,
  SOURCE_MESSAGES,
} from '@/lib/api/operations';
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
    return Object.entries(value).flatMap(([k, v]) => keyPaths(v, prefix ? `${prefix}.${k}` : k));
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

  it('covers every recovery enum in both English and Chinese', () => {
    const states = ['normal', 'idle', 'recovering', 'degraded', 'unknown', 'retired', 'invalid'];
    const reasons = [
      'runtime_live',
      'no_pending_work',
      'runtime_starting',
      'runtime_terminating',
      'runtime_absent',
      'runtime_terminal',
      'runtime_observation_unavailable',
      'runtime_health_degraded',
      'trigger_closed',
      'registration_invalid',
      'configuration_rejected',
    ];
    const runtimes = ['absent', 'starting', 'live', 'terminating', 'terminal', 'unknown'];

    for (const catalog of [en, zh]) {
      expect(Object.keys(catalog.dashboard.detail.recoveryState)).toEqual(states);
      expect(Object.keys(catalog.dashboard.detail.recoveryReason)).toEqual(reasons);
      expect(Object.keys(catalog.dashboard.detail.runtimeState)).toEqual(runtimes);
    }
  });

  // The operations surface renders a localized name for every value the two
  // operations APIs can return. A vocabulary that grew server-side without a
  // catalog entry would render a raw wire string in both languages, so the
  // wire vocabularies themselves are the assertion.
  it('names every operations wire vocabulary in both languages', () => {
    const expected: Array<[keyof typeof en.operations, readonly string[]]> = [
      ['recordKind', ROW_KINDS],
      ['recordKindFilter', RECORD_KINDS],
      ['outcome', OUTCOMES],
      ['delivery', DELIVERY_STATES],
      ['actorKind', ACTOR_KINDS],
      ['principalKind', PRINCIPAL_KINDS],
      ['lifecycleAction', LIFECYCLE_ACTIONS],
      ['sandboxStatus', SANDBOX_STATUSES],
      ['backendKind', SANDBOX_BACKENDS],
      ['attribution', ATTRIBUTION_SOURCES],
      ['warning', SANDBOX_WARNINGS],
      ['sourceHealth', SOURCE_HEALTHS],
      ['sourceMessage', SOURCE_MESSAGES],
      ['errorMessage', OPERATIONS_ERROR_CODES],
    ];
    for (const catalog of [en, zh]) {
      for (const [key, vocabulary] of expected) {
        expect(Object.keys(catalog.operations[key] as Record<string, string>).sort()).toEqual(
          [...vocabulary].sort()
        );
      }
      // Every string is non-empty: a blank cell is indistinguishable from a
      // missing key at runtime.
      expect(
        Object.values(catalog.operations).flatMap((value) =>
          typeof value === 'string' ? [value] : Object.values(value as Record<string, string>)
        )
      ).not.toContain('');
    }
  });

  it('keeps every operations placeholder token identical across languages', () => {
    // The placeholders are substituted by code, so a translated token would
    // silently render the literal `{time}` instead of a value.
    const placeholders = ['ignoredParams', 'queriedAt', 'observedAt', 'remaining', 'errorRequestId'] as const;
    const tokens: Record<(typeof placeholders)[number], string> = {
      ignoredParams: '{names}',
      queriedAt: '{time}',
      observedAt: '{time}',
      remaining: '{duration}',
      errorRequestId: '{id}',
    };
    for (const catalog of [en, zh]) {
      for (const key of placeholders) {
        expect(catalog.operations[key]).toContain(tokens[key]);
      }
    }
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
