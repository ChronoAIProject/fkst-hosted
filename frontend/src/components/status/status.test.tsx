import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StateDot } from './state-dot';
import { StateBadge, type StateBadgeProps, type StateBadgeState } from './state-badge';
import { CiGlyph, type CiStatus } from './ci-glyph';
import { VitalsCell } from './vitals-cell';
import { PostureChip } from './posture-chip';
import { FreshnessChip } from './freshness-chip';

describe('StateDot Unit Tests', () => {
  it('renders correctly with given label and tone color class', () => {
    const { container } = render(<StateDot tone="green" label="Ready for review" />);
    expect(screen.getByText('Ready for review')).toBeInTheDocument();
    const dotEl = container.querySelector('span');
    expect(dotEl?.className).toContain('bg-green');
  });
});

describe('StateBadge Tone Mapping Tests', () => {
  const statesAndTones: { state: StateBadgeState; expectedTone: 'green' | 'red' | 'gold' | 'neutral' }[] = [
    { state: 'thinking', expectedTone: 'neutral' },
    { state: 'ready', expectedTone: 'neutral' },
    { state: 'implementing', expectedTone: 'neutral' },
    { state: 'pr-open', expectedTone: 'neutral' },
    { state: 'reviewing', expectedTone: 'neutral' },
    { state: 'merge-ready', expectedTone: 'neutral' },
    { state: 'merging', expectedTone: 'red' },
    { state: 'fixing', expectedTone: 'neutral' },
    { state: 'review-meta', expectedTone: 'neutral' },
    { state: 'impl-failed', expectedTone: 'red' },
    { state: 'blocked', expectedTone: 'red' },
    { state: 'merged', expectedTone: 'green' },
  ];

  statesAndTones.forEach(({ state, expectedTone }) => {
    it(`maps hyphenated state "${state}" to tone "${expectedTone}" when not pressured`, () => {
      const { container } = render(<StateBadge state={state} />);
      const badgeEl = container.firstElementChild as HTMLElement;
      
      if (expectedTone === 'neutral') {
        expect(badgeEl.style.color).toBe('');
        expect(badgeEl.classList.contains('text-dim')).toBe(true);
      } else {
        expect(badgeEl.style.color).toBe(`var(--${expectedTone})`);
      }
    });
  });

  it('maps reviewing to gold tone under pressure and sets accessibility labels', () => {
    render(<StateBadge state="reviewing" pressure />);
    const badgeEl = screen.getByText('reviewing');
    
    // Gold stays
    expect(badgeEl.style.color).toBe('var(--gold)');
    // Accessibility titles and labels added for colorblind safety
    expect(badgeEl.getAttribute('title')).toBe('reviewing — under pressure');
    expect(badgeEl.getAttribute('aria-label')).toBe('reviewing — under pressure');
  });

  it('maps review-meta to gold tone under pressure and sets accessibility labels', () => {
    render(<StateBadge state="review-meta" pressure />);
    const badgeEl = screen.getByText('review-meta');
    
    expect(badgeEl.style.color).toBe('var(--gold)');
    expect(badgeEl.getAttribute('title')).toBe('review-meta — under pressure');
    expect(badgeEl.getAttribute('aria-label')).toBe('review-meta — under pressure');
  });

  it('renders gated ready badge with dashed border styles in lowercase', () => {
    const { container } = render(<StateBadge state="ready" gated />);
    const badgeEl = container.firstElementChild as HTMLElement;
    expect(badgeEl.className).toContain('border-dashed');
    expect(badgeEl.className).toContain('text-faint');
    expect(badgeEl.textContent).toBe('ready · gated');
  });

  it('ignores gated when state is not ready', () => {
    const { container } = render(<StateBadge {...({ state: 'merged', gated: true } as unknown as StateBadgeProps)} />);
    const badgeEl = container.firstElementChild as HTMLElement;
    
    // Shows standard lowercase "merged" instead of "merged · gated"
    expect(badgeEl.textContent).toBe('merged');
    // Does not suppress the green tone
    expect(badgeEl.style.color).toBe('var(--green)');
  });
});

describe('CiGlyph Unit Tests', () => {
  const cases: { status: CiStatus; expectedChar: string; expectedLabel: string; expectedColor: string }[] = [
    { status: 'passing', expectedChar: '✓', expectedLabel: 'CI passing', expectedColor: 'text-green' },
    { status: 'unknown', expectedChar: '—', expectedLabel: 'CI unknown', expectedColor: 'text-ghost' },
    { status: 'failing', expectedChar: '✗', expectedLabel: 'CI failing', expectedColor: 'text-red' },
  ];

  cases.forEach(({ status, expectedChar, expectedLabel, expectedColor }) => {
    it(`renders correct glyph, class, and role for state "${status}"`, () => {
      render(<CiGlyph status={status} />);
      const el = screen.getByRole('img');
      expect(el.textContent).toBe(expectedChar);
      expect(el.getAttribute('aria-label')).toBe(expectedLabel);
      expect(el.className).toContain(expectedColor);
      expect(el.className).toContain('font-medium');
      expect(el.className).toContain('text-[13px]');
    });
  });
});

describe('VitalsCell Unit Tests', () => {
  it('renders numeric values correctly and drops card chrome', () => {
    const { container } = render(<VitalsCell value={42} label="active runs" />);
    expect(screen.getByText('42')).toBeInTheDocument();
    expect(screen.getByText('active runs')).toBeInTheDocument();
    
    // Ensure card chrome has been dropped
    const rootEl = container.firstElementChild;
    expect(rootEl?.className).not.toContain('rounded-card');
    expect(rootEl?.className).not.toContain('border');
  });

  it("renders 'unknown' as faint 18px text, never 0 or a fake numeral", () => {
    render(<VitalsCell value="unknown" label="median time" />);
    
    // Check that 'unknown' is displayed in faint 18px
    const unknownText = screen.getByText('unknown');
    expect(unknownText).toBeInTheDocument();
    expect(unknownText.className).toContain('text-faint');
    expect(unknownText.className).toContain('text-[18px]');
    
    // Ensure 0 is NOT rendered
    expect(screen.queryByText('0')).toBeNull();
  });

  it('renders labels at 12px size', () => {
    render(<VitalsCell value={5} label="failed" />);
    const labelEl = screen.getByText('failed');
    expect(labelEl.className).toContain('text-[12px]');
  });
});

describe('PostureChip Unit Tests', () => {
  it('renders the fixed string "posture unknown (deploy-time)" only', () => {
    render(<PostureChip />);
    expect(screen.getByText('posture unknown (deploy-time)')).toBeInTheDocument();
  });
});

describe('FreshnessChip Unit Tests', () => {
  it('renders fresh state with normal text', () => {
    const { container } = render(<FreshnessChip source="github" asOf="5m ago" state="fresh" />);
    expect(screen.getByText('github · 5m ago')).toBeInTheDocument();
    expect(container.firstElementChild?.className).toContain('text-faint');
  });

  it('renders syncing state correctly', () => {
    const { container } = render(<FreshnessChip source="github" state="syncing" />);
    expect(screen.getByText('github · syncing…')).toBeInTheDocument();
    expect(container.firstElementChild?.className).toContain('text-faint');
  });

  it('renders stale-warn state with distinct wording and gold border-mix tint style', () => {
    const { container } = render(<FreshnessChip source="github" asOf="5m ago" state="stale-warn" />);
    expect(screen.getByText('github · stale · 5m ago')).toBeInTheDocument();
    
    const chipEl = container.firstElementChild as HTMLElement;
    expect(chipEl.style.color).toBe('var(--gold)');
    expect(chipEl.style.borderColor).toBe('color-mix(in oklab, var(--gold) 38%, var(--line))');
    expect(chipEl.style.backgroundColor).toBe('color-mix(in oklab, var(--gold) 12%, transparent)');
  });

  it('renders stale-critical state with distinct wording and red border-mix tint style', () => {
    const { container } = render(<FreshnessChip source="github" asOf="15m ago" state="stale-critical" />);
    expect(screen.getByText('github · stale 2+ polls · 15m ago')).toBeInTheDocument();
    
    const chipEl = container.firstElementChild as HTMLElement;
    expect(chipEl.style.color).toBe('var(--red)');
    expect(chipEl.style.borderColor).toBe('color-mix(in oklab, var(--red) 38%, var(--line))');
    expect(chipEl.style.backgroundColor).toBe('color-mix(in oklab, var(--red) 12%, transparent)');
  });

  it('renders unknown state with ghost text', () => {
    const { container } = render(<FreshnessChip source="github" state="unknown" />);
    expect(screen.getByText('github · unknown')).toBeInTheDocument();
    expect(container.firstElementChild?.className).toContain('text-ghost');
  });
});
