import { useContent } from '@/i18n';
import { CopyButton } from '@/components/ui/copy-button';
import { StaggerItem } from '@/components/ui/motion';
import type { SessionDetail } from '@/lib/api/types';
import { packageRole } from '@/lib/api/derive';
import { Note, SectionLabel } from './parts';
import { ConfigPanel } from './config-panel';
import { ObserveView } from './observe-view';
import type { ObserveState } from './observe-state';

/** Packages tab: the frozen session configuration (ConfigPanel), then each
 *  declared package decoded to a friendly role + short handle with the full
 *  `owner/repo@ref:path` kept verbatim in a copyable `<code>`. When the Status
 *  tab has already fetched the engine snapshot, the same per-queue activity is
 *  surfaced here too (no second fetch). */
export function TabPackages({
  session,
  observe,
}: {
  session: SessionDetail;
  observe: ObserveState;
}) {
  const d = useContent().dashboard;
  const t = d.detail;

  return (
    <div className="flex flex-col gap-5">
      <ConfigPanel session={session} />

      <section className="flex flex-col gap-2">
        <SectionLabel>
          {d.packages}
          {session.packages.length > 0 && (
            <span className="ml-2 lowercase">· {session.packages.length}</span>
          )}
        </SectionLabel>
        {session.packages.length === 0 ? (
          <Note>{t.packagesNone}</Note>
        ) : (
          <ul className="flex flex-col gap-2">
            {session.packages.map((ref, index) => {
              const decoded = packageRole(ref);
              return (
                // Stagger each row in on the shared curve; collapses to the
                // final state under prefers-reduced-motion (see index.css). Glass
                // package card with a gradient hairline edge + a hover lift so the
                // list reads as a stack of lifted surfaces, not flat rows.
                <StaggerItem
                  key={ref}
                  index={index}
                  className="grad-border hover-lift rounded-card px-3 py-2.5 flex flex-col gap-1 min-w-0 shadow-1"
                >
                  <div className="flex items-baseline gap-2 min-w-0">
                    <span className="font-ui font-semibold text-[13px] text-fg flex-none">
                      {decoded.role}
                    </span>
                    {decoded.role !== decoded.short && (
                      <span className="font-mono text-[11px] text-dim truncate min-w-0">
                        {decoded.short}
                      </span>
                    )}
                  </div>
                  <div className="flex items-start gap-2 min-w-0">
                    <code
                      title={t.packageRefAria}
                      className="font-mono text-[10.5px] text-ghost break-all min-w-0 flex-1"
                    >
                      {ref}
                    </code>
                    <CopyButton value={ref} label={t.packageRefCopy} />
                  </div>
                </StaggerItem>
              );
            })}
          </ul>
        )}
      </section>

      {observe.status === 'loaded' && (
        <section className="flex flex-col gap-2">
          <SectionLabel>{t.queueActivity}</SectionLabel>
          <ObserveView snapshot={observe.snapshot} />
        </section>
      )}
    </div>
  );
}
