/** Shared form-field class recipes (labels above inputs, token-styled). */
// Label nudged one notch brighter (ghost -> faint) for readability on the deeper bg.
export const FIELD_LABEL = 'font-mono text-eyebrow font-medium text-faint uppercase';
// Refined input: a raised surface (was flat bg) that, on focus, lights an amber
// hairline + soft amber glow ring for the elevated "focus glow" feel. The global
// accessible :focus-visible outline is left intact (no outline-none here).
export const FIELD_INPUT =
  'w-full bg-raise border border-line rounded-control px-3 py-2 font-ui text-[13px] text-fg placeholder:text-ghost transition-[border-color,box-shadow] focus:border-[color-mix(in_oklab,var(--amber)_55%,var(--line-2))] focus:shadow-glow-amber';
