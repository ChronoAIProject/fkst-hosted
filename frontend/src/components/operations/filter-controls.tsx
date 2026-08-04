import { useEffect, useId, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { FilterX } from 'lucide-react';
import { cn } from '@/lib/utils';

/**
 * The dense filter toolbar's controls.
 *
 * Closed vocabularies are `<select>`s, so an invalid value cannot be typed at
 * all. Open values are debounced text inputs that COMMIT a validated value — an
 * unparseable entry never reaches the request, it simply leaves the filter unset
 * and marks the field, so a half-typed session id cannot produce a `400` per
 * keystroke.
 *
 * Every control is a real labelled form field. A dense toolbar is exactly where
 * placeholder-only labelling starts to hurt, because the reader is scanning
 * twelve controls, not reading one.
 */

const fieldClass =
  'w-full bg-raise border border-line rounded-control px-2 py-1 font-mono text-[11.5px] text-fg ' +
  'focus:outline-none focus:border-line-2 focus:shadow-glow-amber transition-[border-color,box-shadow]';

function Label({ htmlFor, children }: { htmlFor: string; children: ReactNode }) {
  return (
    <label
      htmlFor={htmlFor}
      className="font-mono text-[9.5px] uppercase tracking-[0.14em] text-ghost"
    >
      {children}
    </label>
  );
}

/** A labelled control cell. `width` keeps the toolbar from reflowing as values
 *  change length. */
function Cell({
  width = 'w-[150px]',
  children,
}: {
  width?: string;
  children: ReactNode;
}) {
  return <div className={cn('flex flex-col gap-1 flex-none', width)}>{children}</div>;
}

/** A closed-vocabulary select. `options` are already localized. */
export function SelectFilter({
  label,
  value,
  options,
  anyLabel,
  onChange,
  width,
  groups,
}: {
  label: string;
  value: string | null;
  /** Flat options; ignored when `groups` is supplied. */
  options?: ReadonlyArray<{ value: string; label: string }>;
  /** Grouped options, for the operation catalog. */
  groups?: ReadonlyArray<{ label: string; options: ReadonlyArray<{ value: string; label: string }> }>;
  anyLabel: string;
  onChange: (next: string | null) => void;
  width?: string;
}) {
  const id = useId();
  return (
    <Cell width={width}>
      <Label htmlFor={id}>{label}</Label>
      <select
        id={id}
        value={value ?? ''}
        onChange={(event) => onChange(event.target.value === '' ? null : event.target.value)}
        className={cn(fieldClass, 'cursor-pointer')}
      >
        <option value="">{anyLabel}</option>
        {groups
          ? groups.map((group) => (
              <optgroup key={group.label} label={group.label}>
                {group.options.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </optgroup>
            ))
          : (options ?? []).map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
      </select>
    </Cell>
  );
}

/** How long a text filter waits before committing. Long enough that typing a
 *  session id is one request, short enough that it still feels live. */
const DEBOUNCE_MS = 400;

/**
 * A debounced, validated text filter.
 *
 * `parse` returns the normalized value or `null`. An empty box clears the
 * filter; a non-empty box that fails `parse` is marked invalid and leaves the
 * committed value alone, so an in-progress entry never fires a request the
 * server would refuse.
 */
export function TextFilter({
  label,
  value,
  parse,
  onCommit,
  width,
  inputMode,
}: {
  label: string;
  value: string | number | null;
  parse: (raw: string) => string | number | null;
  onCommit: (next: string | number | null) => void;
  width?: string;
  inputMode?: 'numeric' | 'text';
}) {
  const id = useId();
  const [draft, setDraft] = useState(value === null ? '' : String(value));
  const [invalid, setInvalid] = useState(false);
  const timer = useRef<number | null>(null);
  // Live mirror so the debounce closure never commits against a stale prop.
  const onCommitRef = useRef(onCommit);
  onCommitRef.current = onCommit;

  // An external change (reset, cross-link, URL navigation) is authoritative over
  // whatever is being typed: the box shows the state, not the other way round.
  const external = value === null ? '' : String(value);
  const lastExternal = useRef(external);
  useEffect(() => {
    if (lastExternal.current !== external) {
      lastExternal.current = external;
      setDraft(external);
      setInvalid(false);
    }
  }, [external]);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );

  const schedule = (raw: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      if (raw.trim() === '') {
        setInvalid(false);
        lastExternal.current = '';
        onCommitRef.current(null);
        return;
      }
      const parsed = parse(raw);
      setInvalid(parsed === null);
      if (parsed !== null) {
        lastExternal.current = String(parsed);
        onCommitRef.current(parsed);
      }
    }, DEBOUNCE_MS);
  };

  return (
    <Cell width={width}>
      <Label htmlFor={id}>{label}</Label>
      <input
        id={id}
        type="text"
        inputMode={inputMode}
        value={draft}
        aria-invalid={invalid || undefined}
        onChange={(event) => {
          setDraft(event.target.value);
          schedule(event.target.value);
        }}
        className={cn(fieldClass, invalid && 'border-red')}
      />
    </Cell>
  );
}

/** A UTC instant picker for the custom range. The control is `datetime-local`
 *  but the value is read as UTC — stated in the label — because an audit window
 *  that silently means "your timezone" is not comparable between two readers. */
export function InstantFilter({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number | null;
  onChange: (next: number | null) => void;
}) {
  const id = useId();
  return (
    <Cell width="w-[186px]">
      <Label htmlFor={id}>{label}</Label>
      <input
        id={id}
        type="datetime-local"
        value={value === null ? '' : toLocalInputValue(value)}
        onChange={(event) => onChange(fromUtcInputValue(event.target.value))}
        className={fieldClass}
      />
    </Cell>
  );
}

/** Epoch-ms → the `YYYY-MM-DDTHH:mm` the input wants, in UTC. */
export function toLocalInputValue(ms: number): string {
  return new Date(ms).toISOString().slice(0, 16);
}

/** The input's `YYYY-MM-DDTHH:mm` → epoch-ms, read as UTC. */
export function fromUtcInputValue(raw: string): number | null {
  if (raw === '') return null;
  const parsed = Date.parse(`${raw}:00Z`);
  return Number.isFinite(parsed) ? parsed : null;
}

/** Clear every filter of the active view back to its default. */
export function ResetFiltersButton({
  label,
  disabled,
  onClick,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1.5 text-dim hover:text-fg hover:shadow-glow-amber transition-[color,box-shadow] cursor-pointer disabled:opacity-50 disabled:cursor-default disabled:hover:text-dim disabled:hover:shadow-none inline-flex items-center gap-1.5 flex-none"
    >
      <FilterX aria-hidden="true" className="w-3 h-3" />
      {label}
    </button>
  );
}
