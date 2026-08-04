import { useRef } from 'react';
import type { KeyboardEvent } from 'react';
import { cn } from '@/lib/utils';

/**
 * A WAI-ARIA tablist with a roving tabindex, matching the session-detail drawer's
 * behaviour exactly: only the selected tab is Tab-reachable, ArrowLeft/ArrowRight
 * wrap around, Home/End jump to the ends, and selection follows focus (automatic
 * activation — both panels are cheap to reveal).
 *
 * Generic over the tab key so the same component drives the page's two views and
 * any future segmented control without a second implementation. The scope
 * control reuses it: a scope switch is exactly the same interaction, and giving
 * it its own keyboard model would be a second thing to get wrong.
 */
export interface TabDefinition<K extends string> {
  key: K;
  label: string;
}

export function Tabs<K extends string>({
  tabs,
  value,
  onChange,
  ariaLabel,
  idBase,
  panelId,
  size = 'md',
  testId,
}: {
  tabs: ReadonlyArray<TabDefinition<K>>;
  value: K;
  onChange: (next: K) => void;
  ariaLabel: string;
  /** Stable id prefix so `aria-controls`/`aria-labelledby` survive re-renders. */
  idBase: string;
  /** The single stable panel every tab controls. */
  panelId: string;
  size?: 'sm' | 'md';
  testId?: string;
}) {
  const refs = useRef<Partial<Record<K, HTMLButtonElement | null>>>({});

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = tabs.findIndex((tab) => tab.key === value);
    let next = index;
    if (event.key === 'ArrowRight') next = (index + 1) % tabs.length;
    else if (event.key === 'ArrowLeft') next = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = tabs.length - 1;
    else return;
    event.preventDefault();
    const target = tabs[next];
    if (!target) return;
    onChange(target.key);
    refs.current[target.key]?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      data-testid={testId}
      onKeyDown={onKeyDown}
      // -1 only makes the container a valid target for the delegated handler;
      // the roving tabindex itself lives on the buttons.
      tabIndex={-1}
      className="flex items-center gap-1 flex-none glass border border-line rounded-control p-1"
    >
      {tabs.map((tab) => (
        <button
          key={tab.key}
          ref={(element) => {
            refs.current[tab.key] = element;
          }}
          type="button"
          role="tab"
          id={`${idBase}-tab-${tab.key}`}
          aria-selected={value === tab.key}
          aria-controls={panelId}
          tabIndex={value === tab.key ? 0 : -1}
          onClick={() => onChange(tab.key)}
          className={cn(
            'font-ui font-semibold rounded-control transition-[color,background-color,box-shadow] duration-150 cursor-pointer whitespace-nowrap',
            size === 'sm' ? 'text-[11.5px] px-2.5 py-1' : 'text-[12.5px] px-3 py-1.5',
            value === tab.key
              ? 'bg-glass-2 text-amber shadow-[var(--shadow-1),var(--glow-amber)]'
              : 'text-dim hover:text-fg'
          )}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
