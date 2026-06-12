
export default {
  title: 'Styles/Fonts',
};

export const Specimen = () => (
  <div className="p-8 bg-bg text-fg min-h-screen">
    <h1 className="text-modal-title font-semibold mb-6">Font Specimen</h1>

    <section className="mb-10 max-w-shell">
      <h2 className="text-xs font-mono text-faint border-b border-line pb-2 mb-4">Space Grotesk (Display Stack)</h2>
      <div className="flex flex-col gap-6 font-display">
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">500 (Medium) - 27px</span>
          <p className="text-[27px] font-medium tracking-[-0.02em] leading-tight">
            FKST Mission Control — 500
          </p>
        </div>
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">600 (SemiBold) - 34px</span>
          <p className="text-[34px] font-semibold tracking-[-0.02em] leading-tight">
            FKST Mission Control — 600
          </p>
        </div>
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">700 (Bold) - 34px</span>
          <p className="text-[34px] font-bold tracking-[-0.02em] leading-tight">
            FKST Mission Control — 700
          </p>
        </div>
      </div>
    </section>

    <section className="mb-10 max-w-shell">
      <h2 className="text-xs font-mono text-faint border-b border-line pb-2 mb-4">IBM Plex Sans (UI Stack)</h2>
      <div className="flex flex-col gap-6 font-ui">
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">400 (Regular) - 14px</span>
          <p className="text-[14px] font-normal leading-normal">
            The visually expressive interface handles complex deployments. (400)
          </p>
        </div>
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">500 (Medium) - 14px (Substituted for 450)</span>
          <p className="text-[14px] font-medium leading-normal">
            The visually expressive interface handles complex deployments. (500)
          </p>
        </div>
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">600 (SemiBold) - 14px</span>
          <p className="text-[14px] font-semibold leading-normal">
            The visually expressive interface handles complex deployments. (600)
          </p>
        </div>
      </div>
      <div className="mt-4 p-3 bg-raise border border-line-2 rounded-control text-xs text-dim">
        <strong>Note:</strong> IBM Plex Sans weight 450 ("Text") is substituted with weight 500 because the static <code>@fontsource</code> packages do not ship a 450 weight.
      </div>
    </section>

    <section className="mb-10 max-w-shell">
      <h2 className="text-xs font-mono text-faint border-b border-line pb-2 mb-4">IBM Plex Mono (Monospace Stack)</h2>
      <div className="flex flex-col gap-6 font-mono text-ghost">
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">400 (Regular) - 12.5px</span>
          <p className="text-[12.5px] font-normal">
            commit e675be43cf7da7e87cd254fbc6d040fdec6d5934 (400)
          </p>
        </div>
        <div>
          <span className="text-xs font-mono text-ghost block mb-1">500 (Medium) - 12.5px</span>
          <p className="text-[12.5px] font-medium">
            commit e675be43cf7da7e87cd254fbc6d040fdec6d5934 (500)
          </p>
        </div>
      </div>
    </section>

    <section className="max-w-shell">
      <h2 className="text-xs font-mono text-faint border-b border-line pb-2 mb-4">Tabular Numbers (A11y/Timestamp align)</h2>
      <div className="grid grid-cols-2 gap-4 max-w-md text-sm">
        <div className="p-4 bg-raise border border-line rounded-panel font-ui">
          <span className="text-xs text-ghost block mb-2">Default (Proportional)</span>
          <div className="flex flex-col gap-1">
            <p>11:11:11</p>
            <p>00:00:00</p>
            <p>99:99:99</p>
          </div>
        </div>
        <div className="p-4 bg-raise border border-line rounded-panel font-ui tabular-nums">
          <span className="text-xs text-ghost block mb-2">Tabular Numerals (.tabular-nums)</span>
          <div className="flex flex-col gap-1">
            <p>11:11:11</p>
            <p>00:00:00</p>
            <p>99:99:99</p>
          </div>
        </div>
      </div>
    </section>
  </div>
);
