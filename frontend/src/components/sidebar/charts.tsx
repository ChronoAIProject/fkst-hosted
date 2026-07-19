import { useReducedMotion } from 'framer-motion';
import {
  Bar,
  BarChart,
  CartesianGrid,
  LabelList,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { useContent } from '@/i18n';
import { foldTail } from '@/lib/api/derive';
import type { ChartRow } from '@/lib/api/derive';
import { FIELD_INPUT } from '@/components/ui/field';
import { cn } from '@/lib/utils';

// Sidebar charts, per the dataviz method: single-series horizontal bars
// (nominal categories → ONE hue per chart, no legend), thin marks (12px,
// 4px rounded data-end, square baseline), hairline solid grid, values
// direct-labeled at every bar tip in text tokens (never the series color).
// Mark hues validated against the raised surface: amber 9.9:1, green 8.3:1
// (≥3:1 required); label/axis text uses --dim at 7.3:1.

const ROW_HEIGHT = 26;
const AXIS_BAND = 12;
const AXIS_WIDTH = 92;
/** Grow-in duration for the bars; brief so a scope change reads as a settle,
 *  not an entrance. Collapsed to instant under reduced motion. */
const BAR_ANIM_MS = 300;
/** Dataviz cap: past ~7 classes the tail folds into "Other". */
const MAX_ROWS = 7;

const HUES = { amber: 'var(--amber)', green: 'var(--green)' } as const;

/** Renders a YAxis category tick as SVG text carrying a native `<title>`, so a
 *  label clipped by the fixed 92px axis width stays recoverable on hover. The
 *  title prefers the row's full `key` (e.g. the whole `owner/repo@ref:path`),
 *  which the axis `label` may have shortened; it falls back to the label. */
function AxisTick(fullByLabel: Map<string, string>) {
  return function Tick({
    x,
    y,
    payload,
  }: {
    // recharts' tick-content props type x/y as string | number; keep the param
    // wide enough to accept that (SVG text x/y take either) so the renderer is
    // assignable to YAxis's `tick` prop.
    x?: number | string;
    y?: number | string;
    payload?: { value?: string | number };
  }) {
    const label = payload?.value == null ? '' : String(payload.value);
    const full = fullByLabel.get(label) ?? label;
    return (
      <text
        x={x}
        y={y}
        dy={3}
        dx={-4}
        textAnchor="end"
        fill="var(--dim)"
        fontSize={10.5}
        fontFamily="var(--mono)"
      >
        {/* Native SVG tooltip — the discoverable fallback for clipped labels. */}
        <title>{full}</title>
        {label}
      </text>
    );
  };
}

function ChartTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: { payload: ChartRow }[];
}) {
  if (!active || !payload?.length) return null;
  const row = payload[0]!.payload;
  return (
    <div className="border border-line rounded-card bg-raise px-2.5 py-1.5 flex items-baseline gap-2">
      <span className="font-mono text-[12px] font-semibold text-fg">{row.value}</span>
      <span className="font-mono text-[10.5px] text-dim break-all">{row.key}</span>
    </div>
  );
}

/** One single-series horizontal bar chart with an eyebrow caption. */
export function CanvasBarChart({
  title,
  rows,
  hue,
}: {
  title: string;
  rows: ChartRow[];
  hue: keyof typeof HUES;
}) {
  const cc = useContent().dashboard.canvas;
  // The bar grow-in is decorative; suppress it entirely under reduced motion so
  // bars snap to their final length (the original instant-render behavior).
  const reduce = useReducedMotion();
  const shown = foldTail(rows, MAX_ROWS, cc.chartOther);
  // Container height includes the axis band so labels never clip (dataviz).
  const height = shown.length * ROW_HEIGHT + AXIS_BAND;
  // Recover the full identity for each (possibly shortened) axis label so the
  // tick tooltip can surface it on hover.
  const fullByLabel = new Map(shown.map((r) => [r.label, r.key]));

  return (
    <figure aria-label={title} className="flex flex-col gap-2 min-w-0">
      <figcaption className="font-mono text-eyebrow text-ghost uppercase">{title}</figcaption>
      {shown.length === 0 ? (
        <p className="font-mono text-[11.5px] text-ghost">{cc.chartEmpty}</p>
      ) : (
        <div style={{ height }} className="w-full">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart
              data={shown}
              layout="vertical"
              margin={{ top: 2, right: 30, bottom: 2, left: 0 }}
            >
              <CartesianGrid horizontal={false} stroke="var(--line)" strokeWidth={1} />
              <XAxis type="number" hide domain={[0, 'dataMax']} allowDecimals={false} />
              <YAxis
                type="category"
                dataKey="label"
                width={AXIS_WIDTH}
                tickLine={false}
                axisLine={{ stroke: 'var(--line)', strokeWidth: 1 }}
                tick={AxisTick(fullByLabel)}
              />
              <Tooltip
                content={<ChartTooltip />}
                cursor={{ fill: 'color-mix(in oklab, var(--raise-2) 55%, transparent)' }}
              />
              <Bar
                dataKey="value"
                fill={HUES[hue]}
                barSize={12}
                radius={[0, 4, 4, 0]}
                isAnimationActive={!reduce}
                animationDuration={BAR_ANIM_MS}
                animationEasing="ease-out"
              >
                <LabelList
                  dataKey="value"
                  position="right"
                  fill="var(--dim)"
                  fontSize={10.5}
                  fontFamily="var(--mono)"
                />
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </div>
      )}
    </figure>
  );
}

/** The one filter row scoping both charts below it (dataviz: filters live in
 *  a single row above the content they scope, never inside a chart card). */
export function ChartScopeSelect({
  id,
  label,
  allLabel,
  options,
  value,
  onChange,
}: {
  id: string;
  label: string;
  allLabel: string;
  options: string[];
  value: string | null;
  onChange: (value: string | null) => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className="font-mono text-eyebrow text-ghost uppercase flex-none">
        {label}
      </label>
      <select
        id={id}
        value={value ?? ''}
        onChange={(e) => onChange(e.target.value || null)}
        className={cn(FIELD_INPUT, 'cursor-pointer py-1.5 text-[12px]')}
      >
        <option value="">{allLabel}</option>
        {options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    </div>
  );
}
