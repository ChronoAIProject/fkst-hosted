import { usePackagesList } from '../../lib/hooks/usePackages';
import { useQueries } from '@tanstack/react-query';
import { getPackage } from '../../lib/api/client';
import { LevelsGrid, LevelsGridCell } from '../../components/layout/levels-grid';
import { SectionHeading } from '../../components/layout/section-heading';
import { HairlineList, HairlineRow } from '../../components/layout/hairline-list';
import { PackageResponse } from '../../lib/api/types';

// Presentational View Component (for easy testing & stories)
export interface PackagesViewProps {
  isLoadingList: boolean;
  listError: string | null;
  packageNames?: string[];
  packagesData?: Record<string, { pkg?: PackageResponse; isLoading?: boolean; error?: unknown }>;
  onAddPackageClick?: () => void;
}

export function PackagesView({
  isLoadingList,
  listError,
  packageNames = [],
  packagesData = {},
  onAddPackageClick,
}: PackagesViewProps) {
  // Compute flat vs composed counts only if ALL details are resolved
  const allResolved =
    packageNames.length > 0 &&
    packageNames.every(
      (name) => packagesData[name] && !packagesData[name].isLoading && packagesData[name].pkg
    );

  let flatCount = 0;
  let composedCount = 0;
  if (allResolved) {
    packageNames.forEach((name) => {
      const detail = packagesData[name];
      if (detail && detail.pkg) {
        if (detail.pkg.composed_deps && detail.pkg.composed_deps.length > 0) {
          composedCount++;
        } else {
          flatCount++;
        }
      }
    });
  }

  return (
    <div className="flex flex-col gap-8 min-w-0">
      {/* SCREEN TOOLBAR */}
      <div className="flex items-center gap-[14px] min-[781px]:gap-4 flex-wrap min-h-[56px] py-2.5 border-b border-line select-none">
        <button
          onClick={onAddPackageClick}
          className="text-[12.5px] font-semibold text-amber-ink bg-amber border-0 rounded-control px-[14px] py-[7px] cursor-pointer hover:brightness-[106%] transition-all flex-none"
        >
          + Add package
        </button>
        <span className="font-mono text-[11px] font-semibold tracking-[0.18em] uppercase text-ghost max-[780px]:hidden">
          Deployment
        </span>
        <div className="flex items-center gap-2 text-[13px] text-dim border border-line rounded-[9px] px-[11px] py-1.5 bg-raise">
          graph <span className="font-mono text-fg font-medium">unknown</span> <span className="text-ghost font-normal">(not exposed by the v1 API)</span>
        </div>
        <div className="flex items-center gap-2 text-[13px] text-dim border border-line rounded-[9px] px-[11px] py-1.5 bg-raise">
          scanned <span className="font-mono text-fg font-medium">unknown</span> <span className="text-ghost font-normal">(not exposed by the v1 API)</span>
        </div>
        <div className="flex items-center gap-2 text-[13px] text-dim opacity-50">
          Disabled packages
          <div className="w-[30px] h-[18px] rounded-[10px] bg-line-2 relative">
            <i className="absolute top-[2px] left-[2px] w-[14px] h-[14px] rounded-full bg-faint" />
          </div>
        </div>
        <span className="ml-auto font-mono text-[11px] text-ghost max-[780px]:hidden">
          {packageNames.length} roots · scan-once
        </span>
      </div>

      {/* INTRO PANEL */}
      <div className="border border-line rounded-panel bg-raise overflow-hidden">
        <div className="p-[18px_22px] text-[13.5px] text-dim leading-[1.62] border-b border-line">
          Packages are the <b className="text-fg font-medium">behavior layer</b> — the Lua that runs on the hosted engine. They live in the <b className="text-fg font-medium">hosted package store</b> (<span className="font-mono text-[12px]">GET/POST /api/v1/packages</span>); an engine <b className="text-fg font-medium">session</b> (<span className="font-mono text-[12px]">POST /api/v1/sessions</span>, one live session per package, lease-fenced) scans its root <b className="text-fg font-medium">once</b> at start and composes <b className="text-fg font-medium">one static graph</b>. Changing the set or topology applies on the next <span className="text-amber">session restart</span> (stop → poll <span className="font-mono text-[12px]">stopped</span> → start) — there is no hot-reload, and the package <b className="text-fg font-medium">source tree is read-only at runtime</b>. What you manage here is <b className="text-fg font-medium">which</b> packages load and how they wire — not their source.
        </div>
        <LevelsGrid className="border-0 rounded-none">
          <LevelsGridCell
            eyebrow="Company"
            value="The deployment"
            description="supervisor + framework + the one composed graph. Receives source events, routes by queue, spawns runs."
          />
          <LevelsGridCell
            eyebrow="Department"
            value="A node"
            description={
              <>
                <code className="text-dim font-mono text-[11px]">departments/&lt;d&gt;/main.lua</code> — declares <code className="text-dim font-mono text-[11px]">consumes</code> / <code className="text-dim font-mono text-[11px]">produces</code>. Nodes = departments.
              </>
            }
          />
          <LevelsGridCell
            eyebrow="Person"
            value="One codex run"
            description="a single codex exec per event. No memory, no identity, exits when done."
          />
        </LevelsGrid>
      </div>

      {/* LOADED PACKAGES SECTION */}
      <div>
        <SectionHeading
          count={
            listError ? (
              <span>package store unreachable — unknown</span>
            ) : isLoadingList ? (
              <span>loading...</span>
            ) : (
              <span>
                <b>{packageNames.length}</b> roots scanned
                {allResolved ? (
                  ` · ${flatCount} flat · ${composedCount} composed · conformance unknown (not exposed by the v1 API)`
                ) : (
                  ' · —'
                )}
              </span>
            )
          }
        >
          Loaded packages
        </SectionHeading>

        <div className="flex items-center gap-[10px] flex-wrap mt-3.5 mb-3.5">
          <button
            onClick={onAddPackageClick}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-dim bg-raise border border-dashed border-line-2 rounded-control px-3.5 py-[7px] cursor-pointer hover:text-fg hover:border-faint hover:border-solid transition-colors"
          >
            <span className="font-mono text-[14px] text-faint">+</span> Add package root
          </button>
          <div className="min-[601px]:ml-auto flex items-center gap-2 flex-wrap max-[600px]:w-full">
            <span className="font-mono text-[11px] text-ghost leading-normal max-[600px]:w-full">
              manage = config + session cycle · <b>not live source edits</b> · <b>v1 grounding:</b> a session runs <b>one composed root</b> (deps come from its composed_deps) — changing the set = create a new package revision (create-only store), then cycle the session
            </span>
            <button
              disabled
              className="text-[12.5px] font-semibold text-amber-ink/50 bg-amber/50 border-0 rounded-control px-3.5 py-[7px] cursor-not-allowed transition-all flex-none"
            >
              Apply changes · stop &amp; restart session
            </button>
          </div>
        </div>

        {/* LIST CONTAINER */}
        {listError ? (
          <div className="bg-raise py-6 px-5 border border-red/40 border-dashed rounded-panel text-center">
            <div className="text-red font-medium text-[14px]">
              package store unreachable — unknown
            </div>
            <div className="text-ghost text-[11px] font-mono mt-1 select-text">
              {listError}
            </div>
          </div>
        ) : isLoadingList ? (
          <HairlineList>
            <PackageRowSkeleton />
            <PackageRowSkeleton />
            <PackageRowSkeleton />
          </HairlineList>
        ) : packageNames.length === 0 ? (
          <HairlineList>
            <div className="bg-raise py-8 px-5 text-center text-dim font-ui select-none">
              No packages loaded. The package store is currently empty.
            </div>
          </HairlineList>
        ) : (
          <HairlineList>
            {packageNames.map((name) => {
              const detail = packagesData[name] || { isLoading: true };
              return (
                <PackageRow
                  key={name}
                  name={name}
                  pkg={detail.pkg}
                  isLoading={detail.isLoading}
                  error={detail.error}
                />
              );
            })}
          </HairlineList>
        )}
      </div>
    </div>
  );
}

// Package Row Presentation Component
export interface PackageRowProps {
  name: string;
  pkg?: PackageResponse;
  isLoading?: boolean;
  error?: unknown;
}

export function PackageRow({ name, pkg, isLoading, error }: PackageRowProps) {
  if (isLoading) {
    return <PackageRowSkeleton name={name} />;
  }

  if (error) {
    return (
      <HairlineRow
        className="grid grid-cols-[minmax(0,1fr)_auto] items-start"
        leftContent={
          <div className="min-w-0">
            <div className="flex items-center gap-[10px] flex-wrap">
              <span className="font-mono text-[14px] font-medium text-fg">{name}</span>
              <span className="font-mono text-[10px] font-semibold tracking-[0.05em] uppercase px-2 py-[3px] rounded-[6px] border bg-raise-2 text-red border-red/40">
                error
              </span>
            </div>
            <div className="text-[12.5px] text-red font-mono mt-1.5">
              Failed to load package details
            </div>
          </div>
        }
      />
    );
  }

  const isComposed = pkg && pkg.composed_deps && pkg.composed_deps.length > 0;

  return (
    <HairlineRow
      className="grid grid-cols-[minmax(0,1fr)_auto] items-start"
      leftContent={
        <div className="min-w-0">
          <div className="flex items-center gap-[10px] flex-wrap">
            <span className="font-mono text-[14px] font-medium text-fg">{name}</span>
            {isComposed ? (
              <span
                style={{ borderColor: 'color-mix(in oklab, var(--amber) 38%, var(--line))' }}
                className="font-mono text-[10px] font-semibold tracking-[0.05em] uppercase px-2 py-[3px] rounded-[6px] border bg-raise-2 text-amber select-none"
              >
                composed
              </span>
            ) : (
              <span className="font-mono text-[10px] font-semibold tracking-[0.05em] uppercase px-2 py-[3px] rounded-[6px] border border-line-2 bg-raise-2 text-dim select-none">
                flat
              </span>
            )}
          </div>

          {/* ROLE LINE - unknown since it's not in the v1 API */}
          <div className="text-[12.5px] text-ghost italic mt-1.5 leading-normal">
            unknown
          </div>

          {/* METADATA LINE - unknown since it's not in the v1 API, with one honest note */}
          <div className="flex flex-wrap gap-x-4 gap-y-1 mt-2.5 text-[11px] text-ghost select-none">
            <span><b>unknown</b> departments</span>
            <span><b>unknown</b> raisers</span>
            <span>conformance <b>unknown</b></span>
            <span>namespace <b>unknown</b></span>
            <span className="text-faint font-normal font-sans">(not exposed by the v1 API)</span>
          </div>

          {/* COMPOSED DEPS CHIPS */}
          {pkg && pkg.composed_deps && pkg.composed_deps.length > 0 && (
            <div className="mt-2.5 flex items-center gap-[7px] flex-wrap text-[11px] text-ghost select-none">
              <span className="text-faint font-normal">composed.deps</span>
              {pkg.composed_deps.map((dep) => (
                <span
                  key={dep}
                  className="px-2 py-0.5 rounded-[6px] border border-line-2 text-dim bg-raise-2"
                >
                  {dep}
                </span>
              ))}
            </div>
          )}
        </div>
      }
      rightContent={
        <div className="flex flex-col items-end gap-3 flex-none select-none">
          <div className="flex items-center gap-[11px]">
            <span className="text-[12px] text-ghost/40 cursor-not-allowed select-none">
              View source ↗ <span className="text-[11px] font-sans font-normal">(source viewer not exposed in v1)</span>
            </span>
          </div>
        </div>
      }
    />
  );
}

// Neutral Loading Skeleton (No pulse animations, name-absent support)
export function PackageRowSkeleton({ name }: { name?: string }) {
  return (
    <div
      data-testid="package-row-skeleton"
      className="bg-raise py-4 px-5 flex justify-between gap-4 min-w-0"
    >
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-[10px] flex-wrap">
          {name ? (
            <span className="font-mono text-[14px] font-medium text-fg">{name}</span>
          ) : (
            <div className="w-24 h-4.5 bg-raise-2 rounded" />
          )}
          <span className="w-12 h-4 bg-raise-2 border border-line-2 rounded-[6px]" />
        </div>
        <div className="w-48 h-3.5 bg-raise-2 rounded mt-2" />
        <div className="w-80 h-3 bg-raise-2 rounded mt-2.5" />
      </div>
      <div className="flex flex-col items-end gap-3 flex-none select-none">
        <span className="text-[12.5px] text-dim/30 font-ui">
          View source ↗
        </span>
      </div>
    </div>
  );
}

// Default Screen Export
export default function PackagesScreen() {
  const { data: names, isLoading: isLoadingList, error: listError } = usePackagesList();

  // Lift per-row details fetching here via useQueries
  const packagesQueries = useQueries({
    queries: (names || []).map((name) => ({
      queryKey: ['packages', name],
      queryFn: () => getPackage(name),
      staleTime: 30000,
      retry: false,
    })),
  });

  // Build packagesData map
  const packagesData: Record<string, { pkg?: PackageResponse; isLoading?: boolean; error?: unknown }> = {};
  (names || []).forEach((name, index) => {
    const query = packagesQueries[index];
    packagesData[name] = {
      pkg: query?.data,
      isLoading: query ? query.isLoading : true,
      error: query ? query.error : undefined,
    };
  });

  return (
    <PackagesView
      isLoadingList={isLoadingList}
      listError={listError ? listError.message : null}
      packageNames={names}
      packagesData={packagesData}
      onAddPackageClick={() => {}}
    />
  );
}
