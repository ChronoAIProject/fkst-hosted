
export default {
  title: 'Styles/Tokens',
};

const COLOR_TOKENS = [
  { name: '--bg', tailwind: 'bg-bg', desc: 'Canvas background' },
  { name: '--raise', tailwind: 'bg-raise', desc: 'Panels, inputs, nav-active' },
  { name: '--raise-2', tailwind: 'bg-raise-2', desc: 'Hover/active raised, chips' },
  { name: '--line', tailwind: 'bg-line', desc: 'Hairline dividers' },
  { name: '--line-2', tailwind: 'bg-line-2', desc: 'Stronger border/outline' },
  { name: '--fg', tailwind: 'bg-fg', textClass: 'text-bg', desc: 'Primary text' },
  { name: '--dim', tailwind: 'bg-dim', textClass: 'text-bg', desc: 'Secondary text' },
  { name: '--faint', tailwind: 'bg-faint', textClass: 'text-bg', desc: 'Tertiary text/labels' },
  { name: '--ghost', tailwind: 'bg-ghost', textClass: 'text-bg', desc: 'Quiet text/meta' },
  { name: '--amber', tailwind: 'bg-amber', textClass: 'text-amber-ink', desc: 'Brand accent' },
  { name: '--amber-ink', tailwind: 'bg-amber-ink', desc: 'Text on amber fills' },
  { name: '--green', tailwind: 'bg-green', textClass: 'text-bg', desc: 'Success semaphore' },
  { name: '--red', tailwind: 'bg-red', textClass: 'text-bg', desc: 'Danger semaphore' },
  { name: '--gold', tailwind: 'bg-gold', textClass: 'text-bg', desc: 'Warning/stale semaphore' },
];

const RADIUS_TOKENS = [
  { name: 'chip', tailwind: 'rounded-chip', value: '6px' },
  { name: 'control', tailwind: 'rounded-control', value: '8px' },
  { name: 'card', tailwind: 'rounded-card', value: '10px' },
  { name: 'panel', tailwind: 'rounded-panel', value: '14px' },
  { name: 'modal', tailwind: 'rounded-modal', value: '16px' },
];

const TYPE_TOKENS = [
  { name: 'eyebrow', tailwind: 'text-eyebrow font-mono uppercase text-ghost', desc: 'WINDOW, DEPLOYMENT' },
  { name: 'body', tailwind: 'text-body font-ui text-fg', desc: 'Standard UI text (14px/1.5)' },
  { name: 'nav', tailwind: 'text-nav font-ui text-fg', desc: 'Nav links (13.5px)' },
  { name: 'modal-title', tailwind: 'text-modal-title font-display font-semibold text-fg', desc: 'Modal headings (19px/-0.01em)' },
];

export const Colors = () => (
  <div className="p-8 bg-bg text-fg min-h-screen">
    <h1 className="text-modal-title font-semibold mb-6">Color Tokens</h1>
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4 max-w-shell">
      {COLOR_TOKENS.map((token) => (
        <div key={token.name} className="flex items-center gap-4 p-4 border border-line bg-raise rounded-panel">
          <div className={`w-16 h-16 rounded-control border border-line-2 flex-none flex items-center justify-center font-semibold text-xs ${token.tailwind} ${token.textClass || 'text-fg'}`}>
            Swatch
          </div>
          <div>
            <code className="font-mono text-sm text-fg font-semibold">{token.name}</code>
            <p className="text-xs text-faint mt-1">{token.desc}</p>
          </div>
        </div>
      ))}
    </div>
  </div>
);

export const BorderRadius = () => (
  <div className="p-8 bg-bg text-fg min-h-screen">
    <h1 className="text-modal-title font-semibold mb-6">Border Radius Scale</h1>
    <div className="flex flex-col gap-6 max-w-shell">
      {RADIUS_TOKENS.map((token) => (
        <div key={token.name} className="flex flex-col gap-2">
          <span className="text-xs font-mono text-faint">{token.name} ({token.value})</span>
          <div className={`p-6 bg-raise border border-line-2 ${token.tailwind}`}>
            <span className="text-sm font-ui text-fg">Sample box styled with {token.tailwind}</span>
          </div>
        </div>
      ))}
    </div>
  </div>
);

export const Typography = () => (
  <div className="p-8 bg-bg text-fg min-h-screen">
    <h1 className="text-modal-title font-semibold mb-6">Type Scale</h1>
    <div className="flex flex-col gap-8 max-w-shell">
      {TYPE_TOKENS.map((token) => (
        <div key={token.name} className="border-b border-line pb-6">
          <span className="text-xs font-mono text-faint mb-2 block">{token.name}</span>
          <p className={token.tailwind}>{token.desc}</p>
        </div>
      ))}
    </div>
  </div>
);

export const Spacing = () => (
  <div className="p-8 bg-bg text-fg min-h-screen">
    <h1 className="text-modal-title font-semibold mb-6">Tailwind default 4px scale (not a locked token set)</h1>
    <div className="flex flex-col gap-4 max-w-shell">
      {[4, 8, 12, 16, 24, 32, 48, 64].map((size) => (
        <div key={size} className="flex items-center gap-4">
          <span className="w-16 font-mono text-xs text-faint">{size}px ({(size / 4)}u)</span>
          <div style={{ width: `${size}px` }} className="h-4 bg-line-2 rounded-chip" />
        </div>
      ))}
    </div>
  </div>
);
