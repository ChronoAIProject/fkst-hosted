import { useState, useEffect } from 'react';
import { usePackagesList } from '../../lib/hooks/usePackages';
import { useQueries } from '@tanstack/react-query';
import { getPackage, ApiError } from '../../lib/api/client';
import { LevelsGrid, LevelsGridCell } from '../../components/layout/levels-grid';
import { SectionHeading } from '../../components/layout/section-heading';
import { HairlineList, HairlineRow } from '../../components/layout/hairline-list';
import { PackageResponse } from '../../lib/api/types';
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '../../components/primitives/select';
import { TriPanel, TriPanelCell } from '../../components/layout/tri-panel';
import { AddPackageModal } from './add-package-modal';
import { Switch } from '../../components/primitives/switch';
import { useSessionRegistry } from '../../lib/hooks/session-registry';
import { useCreateSession, useSession, useStopSession } from '../../lib/hooks/useSessions';
import { isSessionTerminal } from '../../lib/api/truth';
import {
  useDeletePackage,
  useShares,
  useCreateShare,
  useDeleteShare,
} from '../../lib/hooks/usePackageMutations';
import { toast } from '../../components/primitives/toaster';
import {
  Dialog,
  DialogContent,
  DialogClose,
} from '../../components/primitives/dialog';
import { ModalSheet } from '../../components/layout/modal-sheet';

function getErrorMessage(err: unknown, resourceName: string = 'package'): string {
  if (err instanceof ApiError) {
    if (err.status === 403) {
      return 'Action forbidden: you do not have permission (403)';
    }
    if (err.status === 404) {
      return `Action failed: ${resourceName} not found (404)`;
    }
    if (err.status === 409) {
      return `Action failed: ${resourceName} is in use or has active sessions (409)`;
    }
    if (err.status === 503) {
      let msg = err.message || 'LLM gateway error';
      if (!msg.endsWith('(503)')) {
        msg = `${msg} (503)`;
      }
      return msg;
    }
    return err.message || `Request failed with status ${err.status}`;
  }
  const status = err && typeof err === 'object' && ('status' in err || 'statusCode' in err)
    ? (err as { status?: number; statusCode?: number }).status || (err as { status?: number; statusCode?: number }).statusCode
    : undefined;
  const message = err && typeof err === 'object' && 'message' in err
    ? (err as { message?: string }).message
    : undefined;

  if (status === 403) {
    return 'Action forbidden: you do not have permission (403)';
  }
  if (status === 404) {
    return `Action failed: ${resourceName} not found (404)`;
  }
  if (status === 409) {
    return `Action failed: ${resourceName} is in use or has active sessions (409)`;
  }
  if (status === 503) {
    let msg = message || 'LLM gateway error';
    if (!msg.endsWith('(503)')) {
      msg = `${msg} (503)`;
    }
    return msg;
  }
  return message || (err instanceof Error ? err.message : 'An unexpected error occurred');
}


export interface DerivedTopology {
  departments: string[];
  raisers: string[];
}

export function deriveTopology(pkg?: PackageResponse): DerivedTopology {
  if (!pkg || !pkg.files) {
    return { departments: [], raisers: [] };
  }

  const deptsSet = new Set<string>();
  const raisersSet = new Set<string>();

  pkg.files.forEach((f) => {
    const deptMatch = f.path.match(/^departments\/([^/]+)\/main\.lua$/);
    if (deptMatch && deptMatch[1]) {
      deptsSet.add(deptMatch[1]);
    }
    const raiserMatch = f.path.match(/^raisers\/([^/]+)\.lua$/);
    if (raiserMatch && raiserMatch[1]) {
      raisersSet.add(raiserMatch[1]);
    }
  });

  return {
    departments: Array.from(deptsSet).sort(),
    raisers: Array.from(raisersSet).sort(),
  };
}

export function getTopologyEligiblePackages(
  names: string[],
  packagesData: Record<string, { pkg?: PackageResponse; isLoading?: boolean; error?: unknown }>
): string[] {
  return names.filter((name) => {
    const detail = packagesData[name];
    if (!detail || !detail.pkg) return false;
    const { departments } = deriveTopology(detail.pkg);
    return (detail.pkg.composed_deps && detail.pkg.composed_deps.length > 0) || departments.length > 0;
  });
}

// Presentational View Component (for easy testing & stories)
export interface PackagesViewProps {
  isLoadingList: boolean;
  listError: string | null;
  packageNames?: string[];
  packagesData?: Record<string, { pkg?: PackageResponse; isLoading?: boolean; error?: unknown }>;
  onAddPackageClick?: () => void;
  selectedPkgName?: string;
  onSelectedPkgChange?: (name: string) => void;
  sessionStatusCopy?: React.ReactNode;
  isApplyDisabled?: boolean;
  onApplyClick?: () => void;
  cycleState?: 'idle' | 'stopping' | 'polling' | 'creating' | 'error';
  onCancelClick?: () => void;
  onUpdateClick?: (name: string) => void;
  onDeleteClick?: (name: string) => void;
  onSharesClick?: (name: string) => void;
}

export function PackagesView({
  isLoadingList,
  listError,
  packageNames = [],
  packagesData = {},
  onAddPackageClick,
  selectedPkgName = '',
  onSelectedPkgChange,
  sessionStatusCopy,
  isApplyDisabled = true,
  onApplyClick,
  cycleState = 'idle',
  onCancelClick,
  onUpdateClick,
  onDeleteClick,
  onSharesClick,
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

  // Derive list of packages with composed_deps or departments
  const topologyEligiblePackages = getTopologyEligiblePackages(packageNames, packagesData);

  // Selected package's derived raisers and departments
  const selectedPkgDetail = selectedPkgName ? packagesData[selectedPkgName]?.pkg : undefined;
  const { departments: derivedDepts, raisers: derivedRaisers } = deriveTopology(selectedPkgDetail);
  const selectedPkgComposedDeps = selectedPkgDetail?.composed_deps || [];

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
              manage = config + session cycle · <b>not live source edits</b> · <b>v1 grounding:</b> a session runs <b>one composed root</b> (deps come from its composed_deps) — changing the set = create a new package revision, update files, or delete roots via the UI controls below, then cycle the session; per-package enable switches are a target-state UI over that flow
            </span>
            {sessionStatusCopy && (
              <span
                role="status"
                aria-live="polite"
                className="text-[12px] font-mono text-ghost select-text mr-2"
              >
                {sessionStatusCopy}
              </span>
            )}
            {(cycleState === 'stopping' || cycleState === 'polling' || cycleState === 'creating') && onCancelClick && (
              <button
                onClick={onCancelClick}
                className="text-[12.5px] font-semibold rounded-control px-3.5 py-[7px] text-dim bg-raise border border-line-2 hover:border-faint cursor-pointer transition-all flex-none mr-2"
              >
                Cancel
              </button>
            )}
            <button
              disabled={isApplyDisabled}
              onClick={onApplyClick}
              className={`text-[12.5px] font-semibold rounded-control px-3.5 py-[7px] transition-all flex-none border-0 ${
                isApplyDisabled
                  ? 'text-amber-ink/50 bg-amber/50 cursor-not-allowed'
                  : 'text-amber-ink bg-amber cursor-pointer hover:brightness-[106%]'
              }`}
            >
              {selectedPkgName ? `Apply changes to ${selectedPkgName} · stop & restart session` : 'Apply changes · stop & restart session'}
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
                  onUpdateClick={onUpdateClick ? () => onUpdateClick(name) : undefined}
                  onDeleteClick={onDeleteClick ? () => onDeleteClick(name) : undefined}
                  onSharesClick={onSharesClick ? () => onSharesClick(name) : undefined}
                />
              );
            })}
          </HairlineList>
        )}
      </div>

      {/* COMPOSED GRAPH / TOPOLOGY SECTION */}
      <div>
        <SectionHeading count="derived from file paths · scanned at startup">
          Composed graph · topology
        </SectionHeading>

        {/* TOPOLOGY VIEW CONTAINER */}
        <div className="mt-4 border border-line rounded-panel overflow-hidden bg-raise">
          {/* TOPBAR / SELECTOR */}
          <div className="flex items-center gap-2.5 flex-wrap p-[13px_20px] border-b border-line font-mono text-[11.5px] text-ghost">
            <span>active</span>
            {topologyEligiblePackages.length > 0 ? (
              <Select
                value={selectedPkgName}
                onValueChange={onSelectedPkgChange}
              >
                <SelectTrigger
                  aria-label="Active package"
                  className="font-mono text-[11.5px] text-amber border-0 bg-transparent p-0 hover:text-amber/80 h-auto"
                >
                  <SelectValue placeholder="Select package" />
                </SelectTrigger>
                <SelectContent>
                  {topologyEligiblePackages.map((name) => (
                    <SelectItem key={name} value={name}>
                      {name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <span className="text-dim">none</span>
            )}
            <span>· nodes = departments · edges = queues</span>
            
            <span className="min-[981px]:ml-auto flex gap-4 flex-wrap">
              <span><b>amber</b> queue = cross-package</span>
              <span><b>green</b> = terminal</span>
              <span><b>codex</b> = one exec / event</span>
            </span>
          </div>

          {/* SOURCES band */}
          <div className="p-[16px_20px] border-b border-line bg-raise-2/30">
            <div className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost mb-3">
              Sources · raisers · cron —
            </div>
            {derivedRaisers.length > 0 ? (
              <div className="grid grid-cols-1 min-[601px]:grid-cols-2 min-[1081px]:grid-cols-4 gap-2.5">
                {derivedRaisers.map((raiser) => (
                  <div
                    key={raiser}
                    className="min-w-0 border border-line rounded-card p-[11px_13px] bg-raise"
                  >
                    <div className="font-mono text-[12px] text-fg break-all">
                      {raiser}
                    </div>
                    <div className="inline-flex items-center gap-1.5 mt-2 font-mono text-[10.5px] text-faint">
                      <span className="w-1.5 h-1.5 rounded-full bg-amber flex-none" />
                      cadence — <span className="text-[9.5px] text-ghost normal-case">(declared in Lua, not parsed)</span>
                    </div>
                    <div className="font-mono text-[10.5px] text-ghost mt-1.5 break-all">
                      → unknown <span className="text-[9.5px] text-ghost normal-case">(not parsed)</span>
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="text-[12.5px] text-dim font-mono italic">
                No raisers derived.
              </div>
            )}
          </div>

          {/* PIPELINE / DEPARTMENTS */}
          <div className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost p-[14px_20px_4px] flex items-center justify-between flex-wrap gap-2">
            <span>Departments · pipeline order · consumes → produces</span>
            <span className="normal-case font-normal text-ghost italic select-none">
              (wiring declared in Lua; not parsed by this console)
            </span>
          </div>
          <div className="p-[6px_20px_18px] flex flex-col">
            {derivedDepts.length > 0 ? (
              derivedDepts.map((dept) => (
                <div
                  key={dept}
                  className="grid grid-cols-1 min-[781px]:grid-cols-[140px_minmax(0,1fr)_auto] min-[981px]:grid-cols-[170px_minmax(0,1fr)_auto] gap-3.5 items-start py-[13px] border-t border-line/60 first:border-t-0"
                >
                  <div className="flex flex-col gap-1 min-w-0">
                    <span className="font-mono text-[12.5px] text-fg break-all">
                      {dept}
                    </span>
                    <span className="font-mono font-semibold text-[9.5px] tracking-[0.04em] uppercase text-gold w-max">
                      wiring unknown
                    </span>
                  </div>
                  
                  <div className="min-w-0 flex flex-col gap-1.5">
                    <div className="flex items-center gap-2 flex-wrap min-w-0">
                      <span className="font-mono text-[10px] text-ghost w-[62px] flex-none tracking-[0.04em] uppercase">
                        consumes
                      </span>
                      <span className="font-mono text-[11px] text-dim px-2 py-0.5 rounded-[6px] border border-line bg-raise-2 whitespace-nowrap select-none max-w-full overflow-hidden text-ellipsis">
                        unknown
                      </span>
                    </div>
                    <div className="flex items-center gap-2 flex-wrap min-w-0">
                      <span className="font-mono text-[10px] text-ghost w-[62px] flex-none tracking-[0.04em] uppercase">
                        produces
                      </span>
                      <span className="font-mono text-[11px] text-dim px-2 py-0.5 rounded-[6px] border border-line bg-raise-2 whitespace-nowrap select-none max-w-full overflow-hidden text-ellipsis">
                        unknown
                      </span>
                    </div>
                  </div>

                  <span className="font-mono text-[10.5px] text-ghost px-[9px] py-[3px] rounded-[6px] border border-line whitespace-nowrap select-none w-max max-[780px]:mt-2">
                    unknown
                  </span>
                </div>
              ))
            ) : (
              <div className="text-[12.5px] text-dim font-mono italic py-2">
                No departments derived.
              </div>
            )}
          </div>

          {/* FOOTNOTE */}
          <div className="p-[13px_20px] border-t border-line font-mono text-[11.5px] text-ghost leading-relaxed select-none">
            {selectedPkgName && (
              <div className="mb-1 text-fg font-medium">
                {selectedPkgName}
                {selectedPkgComposedDeps.length > 0 ? ` + ${selectedPkgComposedDeps.join(' + ')}` : ''}
                {' → one composed graph'}
              </div>
            )}
            <div>
              edges are queues when declared by Lua; queue wiring is parsed only by the engine at session start — not exposed by the v1 API
            </div>
          </div>
        </div>
      </div>



      {/* READ / WRITE BOUNDARY SECTION */}
      <div>
        <SectionHeading count="the non-negotiable — what is read-only, what the FE manages, where writes land">
          Read / write boundary
        </SectionHeading>

        <TriPanel className="mt-4">
          <TriPanelCell
            dotClassName="bg-faint"
            header="Read-only"
            title="Package source tree"
            body={
              <>
                The loaded Lua is{' '}
                <b className="text-fg font-medium">read-only at runtime</b> —
                even to the engine. Departments have no lifecycle hooks, no
                shared memory, no persistent state. The graph is scanned{' '}
                <code className="font-mono text-[11.5px] text-faint">once</code>{' '}
                at startup.
              </>
            }
            tagSlot={
              <span className="font-mono text-[11px] px-[9px] py-[3px] rounded-[7px] border border-line-2 text-faint select-none">
                runtime read-only
              </span>
            }
          />
          <TriPanelCell
            dotClassName="bg-amber"
            header="FE manages"
            title="Which packages load · topology · posture"
            body={
              <>
                This console sets the package{' '}
                <b className="text-fg font-medium">set</b>, the composed
                topology, and the global posture. Changes are{' '}
                <b className="text-fg font-medium">config</b>, applied on the
                next supervise{' '}
                <code className="font-mono text-[11.5px] text-faint">restart</code>{' '}
                — never a live source edit, never per-goal.
              </>
            }
            tagSlot={
              <span
                style={{ borderColor: 'color-mix(in oklab, var(--amber) 35%, var(--line))' }}
                className="font-mono text-[11px] px-[9px] py-[3px] rounded-[7px] border text-amber select-none"
              >
                applied via restart
              </span>
            }
          />
          <TriPanelCell
            dotClassName="bg-red"
            header="Business writes"
            title="GitHub only · under REAL posture"
            body={
              <>
                Issues, PRs, comments, merges land on{' '}
                <b className="text-fg font-medium">GitHub</b> — only when{' '}
                <code className="font-mono text-[11.5px] text-faint">FKST_GITHUB_WRITE</code>{' '}
                is REAL (global, never per-goal). The redb durable-delivery
                transport is{' '}
                <b className="text-fg font-medium">engine-internal</b>, not a
                business fact store.
              </>
            }
            tagSlot={
              <span
                style={{ borderColor: 'color-mix(in oklab, var(--red) 40%, var(--line))' }}
                className="font-mono text-[11px] px-[9px] py-[3px] rounded-[7px] border text-red select-none"
              >
                REAL posture required
              </span>
            }
          />
        </TriPanel>

        {/* FOOTER */}
        <div className="mt-8 pt-3.5 border-t border-line flex gap-6 font-mono text-[11px] text-ghost flex-wrap select-none">
          <span>package set &amp; topology <b>poll-derived</b></span>
          <span>the graph is parsed by the hosted backend from the <b>loaded package roots</b></span>
          <span>scan-once at session start · changes apply on <span className="text-gold">session restart</span> · session: pending → validating → running → stopping → stopped / failed</span>
          <span>state as of <b>unknown — not exposed by the v1 API</b></span>
        </div>
      </div>
    </div>
  );
}

export interface PackageRowProps {
  name: string;
  pkg?: PackageResponse;
  isLoading?: boolean;
  error?: unknown;
  onUpdateClick?: () => void;
  onDeleteClick?: () => void;
  onSharesClick?: () => void;
}

export function PackageRow({ name, pkg, isLoading, error, onUpdateClick, onDeleteClick, onSharesClick }: PackageRowProps) {
  const [isEnabled, setIsEnabled] = useState(true);

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
            <span data-testid="pkg-name" className="font-mono text-[14px] font-medium text-fg">{name}</span>
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
        <div className="flex flex-col items-end gap-2.5 flex-none select-none">
          <div className="flex items-center gap-2">
            <span className={`font-mono text-[11px] min-w-[46px] text-right transition-colors ${isEnabled ? 'text-amber' : 'text-ghost'}`}>
              {isEnabled ? 'enabled' : 'disabled'}
            </span>
            <Switch
              checked={isEnabled}
              onCheckedChange={(checked) => {
                setIsEnabled(checked);
              }}
              aria-label={`Toggle target state for ${name}`}
            />
          </div>
          <span className="text-[10px] text-gold font-mono max-w-[150px] text-right leading-tight">
            target state — applies via session restart; no enable endpoint in v1
          </span>
          <div className="flex items-center gap-[11px] mt-1">
            {onUpdateClick && (
              <button
                onClick={onUpdateClick}
                className="text-[12px] text-amber hover:text-amber/80 font-semibold cursor-pointer transition-colors"
              >
                Update
              </button>
            )}
            {onUpdateClick && onDeleteClick && <span className="text-ghost">·</span>}
            {onDeleteClick && (
              <button
                onClick={onDeleteClick}
                className="text-[12px] text-red hover:text-red/80 font-semibold cursor-pointer transition-colors"
              >
                Delete
              </button>
            )}
            {(onUpdateClick || onDeleteClick) && onSharesClick && <span className="text-ghost">·</span>}
            {onSharesClick && (
              <button
                onClick={onSharesClick}
                className="text-[12px] text-amber hover:text-amber/80 font-semibold cursor-pointer transition-colors"
              >
                Shares
              </button>
            )}
            {(onUpdateClick || onDeleteClick || onSharesClick) && <span className="text-ghost">·</span>}
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

export interface DeletePackageModalProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  packageName: string;
  onConfirm: () => Promise<void>;
  isDeleting: boolean;
  error: string | null;
}

export function DeletePackageModal({
  isOpen,
  onOpenChange,
  packageName,
  onConfirm,
  isDeleting,
  error,
}: DeletePackageModalProps) {
  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent className="p-0" showClose={false}>
        <ModalSheet
          title="Delete package"
          meta={
            <span>
              permanently remove from store — <span className="font-mono text-[11.5px] text-ghost">DELETE /api/v1/packages/{packageName}</span>
            </span>
          }
          closeButtonSlot={
            <DialogClose
              aria-label="Close"
              className="w-[30px] h-[30px] rounded-control border border-line bg-raise-2 text-faint hover:text-fg hover:border-faint flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
            >
              <span className="text-[17px] leading-none" aria-hidden="true">✕</span>
            </DialogClose>
          }
          actionButtonsSlot={
            <>
              <button
                type="button"
                onClick={() => onOpenChange(false)}
                className="text-[12.5px] font-medium border border-line-2 bg-raise-2 text-dim rounded-control px-3.5 py-2 cursor-pointer hover:border-faint hover:text-fg transition-colors select-none"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={onConfirm}
                disabled={isDeleting}
                className="text-[12.5px] font-semibold text-red bg-red/10 border border-red/30 rounded-control px-4 py-2 cursor-pointer hover:bg-red/20 transition-all disabled:opacity-50 select-none"
              >
                {isDeleting ? 'Deleting...' : 'Delete package'}
              </button>
            </>
          }
        >
          <div className="flex flex-col gap-4">
            {error && (
              <div className="bg-red/10 border border-red/30 rounded-control p-3 text-red text-[13px] font-mono select-text" role="alert">
                {error}
              </div>
            )}
            <p className="text-[13.5px] leading-normal text-dim">
              Are you sure you want to permanently delete the package <b className="text-fg font-medium font-mono">{packageName}</b>? This action cannot be undone.
            </p>
          </div>
        </ModalSheet>
      </DialogContent>
    </Dialog>
  );
}

export function PackageSharesPanel({ packageName }: { packageName: string }) {
  const { data: shares, isLoading, error } = useShares(packageName);
  const createShareMutation = useCreateShare();
  const deleteShareMutation = useDeleteShare();

  const [granteeId, setGranteeId] = useState('');
  const [granteeKind, setGranteeKind] = useState<'user' | 'org'>('user');
  const [level, setLevel] = useState<'read' | 'use'>('read');
  const [actionError, setActionError] = useState<string | null>(null);

  // Reset local state when package changes
  useEffect(() => {
    setGranteeId('');
    setActionError(null);
  }, [packageName]);

  const handleCreateShare = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!granteeId.trim()) return;
    setActionError(null);
    try {
      await createShareMutation.mutateAsync({
        name: packageName,
        share: {
          grantee_id: granteeId.trim(),
          grantee_kind: granteeKind,
          level,
        },
      });
      setGranteeId('');
      toast({
        title: 'Share granted',
        description: `Successfully shared ${packageName} with ${granteeId}`,
      });
    } catch (err) {
      setActionError(getErrorMessage(err));
    }
  };

  const handleDeleteShare = async (shareId: string, granteeId: string) => {
    setActionError(null);
    try {
      await deleteShareMutation.mutateAsync({
        name: packageName,
        shareId,
      });
      toast({
        title: 'Share revoked',
        description: `Successfully revoked share for ${granteeId}`,
      });
    } catch (err) {
      setActionError(getErrorMessage(err, 'share'));
    }
  };

  return (
    <div className="flex flex-col gap-4">

      <div className="p-5 flex flex-col gap-4">
        {actionError && (
          <div className="bg-red/10 border border-red/30 rounded-control p-3 text-red text-[13px] font-mono select-text" role="alert">
            {actionError}
          </div>
        )}

        {/* Existing Shares List */}
        <div>
          <div className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost mb-2">
            Active Grants
          </div>
          {isLoading ? (
            <div className="text-[12.5px] text-ghost font-mono">Loading shares...</div>
          ) : error ? (
            <div className="text-[12.5px] text-red font-mono">Failed to load shares</div>
          ) : !shares || shares.length === 0 ? (
            <div className="text-[12.5px] text-dim font-mono italic">No shares granted yet.</div>
          ) : (
            <div className="flex flex-col gap-2">
              {shares.map((share) => (
                <div
                  key={share.id}
                  className="flex items-center justify-between border border-line rounded-card p-3 bg-raise-2"
                >
                  <div className="flex items-center gap-2.5 flex-wrap min-w-0">
                    <span className="font-mono text-[12.5px] text-fg break-all font-medium">
                      {share.grantee_id}
                    </span>
                    <span className="font-mono text-[10px] uppercase px-1.5 py-0.5 rounded border border-line-2 bg-raise text-dim">
                      {share.grantee_kind}
                    </span>
                    <span className="font-mono text-[10px] uppercase px-1.5 py-0.5 rounded border border-amber/30 bg-raise text-amber font-semibold">
                      {share.level}
                    </span>
                    <span className="text-[11px] text-ghost">
                      by {share.granted_by}
                    </span>
                  </div>
                  <button
                    onClick={() => handleDeleteShare(share.id, share.grantee_id)}
                    className="text-[12px] text-red hover:text-red-ink font-semibold cursor-pointer transition-colors ml-4"
                  >
                    Revoke
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Grant Share Form */}
        <form onSubmit={handleCreateShare} className="border-t border-line/60 pt-4 mt-2">
          <div className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost mb-3">
            Grant New Share
          </div>
          <div className="grid grid-cols-1 md:grid-cols-[1fr_auto_auto_auto] gap-3 items-end">
            <div className="flex flex-col gap-1.5 min-w-0">
              <label htmlFor="grantee-id-input" className="text-[10px] font-mono text-ghost select-none">
                Grantee ID (User or Org Name)
              </label>
              <input
                id="grantee-id-input"
                type="text"
                value={granteeId}
                onChange={(e) => setGranteeId(e.target.value)}
                placeholder="e.g. user123 or org-name"
                className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2 outline-none w-full focus:border-faint transition-colors"
              />
            </div>

            <div className="flex flex-col gap-1.5 flex-none">
              <label htmlFor="grantee-kind-select" className="text-[10px] font-mono text-ghost select-none">
                Kind
              </label>
              <select
                id="grantee-kind-select"
                value={granteeKind}
                onChange={(e) => setGranteeKind(e.target.value as 'user' | 'org')}
                className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2 outline-none focus:border-faint transition-colors"
              >
                <option value="user">User</option>
                <option value="org">Organization</option>
              </select>
            </div>

            <div className="flex flex-col gap-1.5 flex-none">
              <label htmlFor="share-level-select" className="text-[10px] font-mono text-ghost select-none">
                Level
              </label>
              <select
                id="share-level-select"
                value={level}
                onChange={(e) => setLevel(e.target.value as 'read' | 'use')}
                className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2 outline-none focus:border-faint transition-colors"
              >
                <option value="read">Read</option>
                <option value="use">Use</option>
              </select>
            </div>

            <button
              type="submit"
              disabled={!granteeId.trim() || createShareMutation.isPending}
              className="text-[12.5px] font-semibold text-amber-ink bg-amber border-0 rounded-control px-4 py-2 cursor-pointer hover:brightness-[106%] transition-all disabled:opacity-50 flex-none h-[38px]"
            >
              Grant
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

export default function PackagesScreen() {
  const { data: names, isLoading: isLoadingList, error: listError } = usePackagesList();
  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [pkgToUpdate, setPkgToUpdate] = useState<PackageResponse | null>(null);
  const [pkgToDeleteName, setPkgToDeleteName] = useState<string | null>(null);
  const [pkgToShareName, setPkgToShareName] = useState<string | null>(null);

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

  // Derive list of packages with composed_deps or departments
  const topologyEligiblePackages = getTopologyEligiblePackages(names || [], packagesData);

  const [selectedPkgName, setSelectedPkgName] = useState<string>('');

  useEffect(() => {
    if (!selectedPkgName && topologyEligiblePackages.length > 0) {
      setSelectedPkgName(topologyEligiblePackages[0] || '');
    }
  }, [topologyEligiblePackages, selectedPkgName]);

  // W2.F4: Session Cycling Flow
  const { getSessionId } = useSessionRegistry();
  const sessionId = selectedPkgName ? getSessionId(selectedPkgName) : undefined;

  const stopSessionMutation = useStopSession();
  const createSessionMutation = useCreateSession();

  const [cycleState, setCycleState] = useState<'idle' | 'stopping' | 'polling' | 'creating' | 'error'>('idle');
  const [cycleError, setCycleError] = useState<string | null>(null);

  // Reset states when changing active package
  useEffect(() => {
    setCycleState('idle');
    setCycleError(null);
  }, [selectedPkgName]);

  // Poll status of the stopping session
  const sessionQuery = useSession(cycleState === 'polling' ? sessionId : undefined);

  useEffect(() => {
    if (cycleState === 'polling' && sessionQuery.data?.status) {
      const status = sessionQuery.data.status;
      if (isSessionTerminal(status)) {
        setCycleState('creating');
        createSessionMutation.mutate(selectedPkgName, {
          onSuccess: () => {
            setCycleState('idle');
          },
          onError: (err: unknown) => {
            setCycleState('error');
            const apiErr = err as { status?: number; statusCode?: number; message?: string };
            if (apiErr && (apiErr.status === 409 || apiErr.statusCode === 409)) {
              setCycleError("session stopped, but restart failed — package already has a live session; its id isn't exposed by the v1 API, so it can't be stopped from here.");
            } else {
              setCycleError(apiErr.message || 'Failed to create new session');
            }
          },
        });
      }
    }
  }, [cycleState, sessionQuery.data?.status, selectedPkgName, createSessionMutation]);

  // Handle polling errors
  useEffect(() => {
    if (cycleState === 'polling' && sessionQuery.isError) {
      setCycleState('error');
      setCycleError('status poll failed — session may still be stopping; Apply to retry');
    }
  }, [cycleState, sessionQuery.isError]);

  const handleApplyClick = async () => {
    if (!sessionId) return;
    setCycleError(null);
    setCycleState('stopping');
    try {
      await stopSessionMutation.mutateAsync(sessionId);
      setCycleState('polling');
    } catch (err: unknown) {
      setCycleState('error');
      const apiErr = err as { message?: string };
      setCycleError(apiErr.message || 'Failed to stop session');
    }
  };

  // Construct status copy
  let sessionStatusCopy: React.ReactNode = null;
  if (cycleState === 'stopping') {
    sessionStatusCopy = <span className="text-ghost">{selectedPkgName} · stop requested (202 ack) →</span>;
  } else if (cycleState === 'polling') {
    sessionStatusCopy = <span className="text-ghost">{selectedPkgName} · waiting for stopped →</span>;
  } else if (cycleState === 'creating') {
    sessionStatusCopy = <span className="text-ghost">{selectedPkgName} · starting new session →</span>;
  } else if (cycleState === 'error') {
    if (cycleError?.includes("package already has a live session")) {
      sessionStatusCopy = (
        <span className="text-red font-mono text-[11px] leading-tight">
          {selectedPkgName} · session stopped, but restart failed — package already has a live session; its id isn't exposed by the v1 API, so it can't be stopped from here.
        </span>
      );
    } else {
      sessionStatusCopy = <span className="text-red font-mono text-[11px] leading-tight">{selectedPkgName} · {cycleError}</span>;
    }
  } else if (!sessionId && selectedPkgName) {
    sessionStatusCopy = (
      <span className="text-gold font-mono text-[11px] leading-tight">
        {selectedPkgName} · current session id not exposed by the v1 API — this console can only manage sessions it started this tab.
      </span>
    );
  }

  const isApplyDisabled =
    isLoadingList ||
    !selectedPkgName ||
    !sessionId ||
    (cycleState !== 'idle' && cycleState !== 'error');

  const deleteMutation = useDeletePackage();
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const handleDeleteConfirm = async () => {
    if (!pkgToDeleteName) return;
    setDeleteError(null);
    try {
      await deleteMutation.mutateAsync(pkgToDeleteName);
      toast({
        title: 'Deleted',
        description: `Successfully deleted package ${pkgToDeleteName}`,
      });
      setPkgToDeleteName(null);
    } catch (err) {
      setDeleteError(getErrorMessage(err, 'package'));
    }
  };

  return (
    <>
      <PackagesView
        isLoadingList={isLoadingList}
        listError={listError ? listError.message : null}
        packageNames={names}
        packagesData={packagesData}
        onAddPackageClick={() => {
          setPkgToUpdate(null);
          setIsAddModalOpen(true);
        }}
        selectedPkgName={selectedPkgName}
        onSelectedPkgChange={setSelectedPkgName}
        sessionStatusCopy={sessionStatusCopy}
        isApplyDisabled={isApplyDisabled}
        onApplyClick={handleApplyClick}
        cycleState={cycleState}
        onCancelClick={() => {
          setCycleState('idle');
          setCycleError(null);
        }}
        onUpdateClick={(name) => {
          const pkg = packagesData[name]?.pkg;
          if (pkg) {
            setPkgToUpdate(pkg);
          }
        }}
        onDeleteClick={(name) => {
          setPkgToDeleteName(name);
        }}
        onSharesClick={(name) => {
          setPkgToShareName(name);
        }}
      />
      <AddPackageModal
        isOpen={isAddModalOpen || !!pkgToUpdate}
        onOpenChange={(open) => {
          if (!open) {
            setIsAddModalOpen(false);
            setPkgToUpdate(null);
          }
        }}
        mode={pkgToUpdate ? 'update' : 'create'}
        packageName={pkgToUpdate?.name}
        initialFiles={pkgToUpdate?.files}
        initialComposedDeps={pkgToUpdate?.composed_deps}
      />
      <DeletePackageModal
        isOpen={!!pkgToDeleteName}
        onOpenChange={(open) => {
          if (!open) {
            setPkgToDeleteName(null);
            setDeleteError(null);
          }
        }}
        packageName={pkgToDeleteName || ''}
        onConfirm={handleDeleteConfirm}
        isDeleting={deleteMutation.isPending}
        error={deleteError}
      />
      <SharesModal
        isOpen={!!pkgToShareName}
        onOpenChange={(open) => {
          if (!open) {
            setPkgToShareName(null);
          }
        }}
        packageName={pkgToShareName || ''}
      />
    </>
  );
}

export interface SharesModalProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  packageName: string;
}

export function SharesModal({ isOpen, onOpenChange, packageName }: SharesModalProps) {
  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent className="p-0" showClose={false}>
        <ModalSheet
          title="Manage shares"
          meta={
            <span>
              sharing controls for package <span className="font-mono text-[11.5px] font-bold text-fg">{packageName}</span>
            </span>
          }
          closeButtonSlot={
            <DialogClose
              aria-label="Close"
              className="w-[30px] h-[30px] rounded-control border border-line bg-raise-2 text-faint hover:text-fg hover:border-faint flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
            >
              <span className="text-[17px] leading-none" aria-hidden="true">✕</span>
            </DialogClose>
          }
        >
          <PackageSharesPanel packageName={packageName} />
        </ModalSheet>
      </DialogContent>
    </Dialog>
  );
}
