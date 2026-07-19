import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { ReactNode } from 'react';
import { render, screen } from '@testing-library/react';
import type { ChartRow } from '@/lib/api/derive';

// ---- Mocks ------------------------------------------------------------------
// reduced-motion is a mutable knob so a single suite can exercise both the
// animated and the instant-render branches without remounting the module.
const motionState = { reduced: false };
vi.mock('framer-motion', () => ({ useReducedMotion: () => motionState.reduced }));

// recharts cannot measure itself under jsdom, and this suite only cares about
// the props the chart hands to <Bar> and the tick renderer it hands to <YAxis>.
// So we replace recharts with thin capture stubs: passthrough containers, a Bar
// that records its animation props, and a YAxis that records its `tick` fn.
const barProps: Record<string, unknown>[] = [];
let capturedTick: ((p: unknown) => ReactNode) | null = null;

vi.mock('recharts', () => {
  const Passthrough = ({ children }: { children?: ReactNode }) => <div>{children}</div>;
  return {
    ResponsiveContainer: Passthrough,
    BarChart: Passthrough,
    CartesianGrid: () => null,
    XAxis: () => null,
    Tooltip: () => null,
    LabelList: () => null,
    Bar: ({ children, ...rest }: { children?: ReactNode }) => {
      barProps.push(rest);
      return <div data-testid="bar">{children}</div>;
    },
    YAxis: ({ tick }: { tick?: (p: unknown) => ReactNode }) => {
      capturedTick = tick ?? null;
      return null;
    },
  };
});

import { CanvasBarChart } from './charts';

const ROWS: ChartRow[] = [
  // `label` is the human-short tail; `key` is the full recoverable identity.
  { key: 'octo/repo@main:packages/very-long-triage-package', label: 'very-long-triage-package', value: 5 },
  { key: 'octo/repo@main:packages/build', label: 'build', value: 2 },
];

beforeEach(() => {
  barProps.length = 0;
  capturedTick = null;
  motionState.reduced = false;
});

describe('CanvasBarChart bar animation (reduced-motion gate)', () => {
  it('enables a brief grow-in when motion is allowed', () => {
    render(<CanvasBarChart title="Sessions" rows={ROWS} hue="amber" />);

    expect(barProps).toHaveLength(1);
    expect(barProps[0]!.isAnimationActive).toBe(true);
    expect(barProps[0]!.animationDuration).toBe(300);
    expect(barProps[0]!.animationEasing).toBe('ease-out');
  });

  it('renders instantly (no animation) under reduced motion', () => {
    motionState.reduced = true;
    render(<CanvasBarChart title="Sessions" rows={ROWS} hue="green" />);

    expect(barProps).toHaveLength(1);
    // The original snap-on-mount behavior must survive for reduced-motion users.
    expect(barProps[0]!.isAnimationActive).toBe(false);
  });
});

describe('CanvasBarChart axis tick tooltip (recoverable labels)', () => {
  it('renders a native <title> carrying the full key so a clipped label is recoverable', () => {
    render(<CanvasBarChart title="Sessions" rows={ROWS} hue="amber" />);

    expect(capturedTick).toBeTypeOf('function');
    // Render the captured tick for the shortened label and assert the tooltip
    // surfaces the full, un-clipped identity.
    render(<svg>{capturedTick!({ x: 10, y: 20, payload: { value: 'very-long-triage-package' } })}</svg>);

    const title = document.querySelector('title');
    expect(title?.textContent).toBe('octo/repo@main:packages/very-long-triage-package');
    // The visible text stays the short label; only hover recovers the rest.
    expect(screen.getByText('very-long-triage-package')).toBeInTheDocument();
  });

  it('falls back to the label itself when no fuller identity is known', () => {
    render(<CanvasBarChart title="Sessions" rows={ROWS} hue="amber" />);
    expect(capturedTick).toBeTypeOf('function');

    // A label absent from the row set (e.g. the folded "Other" bucket has its
    // own key, but an unknown label proves the fallback path) → title === label.
    render(<svg>{capturedTick!({ x: 0, y: 0, payload: { value: 'unknown-label' } })}</svg>);

    const title = document.querySelector('title');
    expect(title?.textContent).toBe('unknown-label');
  });

  it('tolerates a missing tick payload value without throwing', () => {
    render(<CanvasBarChart title="Sessions" rows={ROWS} hue="amber" />);
    expect(capturedTick).toBeTypeOf('function');

    expect(() => render(<svg>{capturedTick!({ x: 0, y: 0, payload: {} })}</svg>)).not.toThrow();
  });
});

describe('CanvasBarChart empty state', () => {
  it('shows the empty caption and no bar when there are no rows', () => {
    render(<CanvasBarChart title="Sessions" rows={[]} hue="amber" />);

    expect(screen.getByText('Nothing to chart yet.')).toBeInTheDocument();
    // No chart is rendered, so no Bar props were captured.
    expect(barProps).toHaveLength(0);
  });
});
