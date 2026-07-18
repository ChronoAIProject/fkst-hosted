import { useEffect, useMemo, useRef } from 'react';
import { ReactFlow, useReactFlow } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useContent } from '@/i18n';
import type { AccountOverview, RepoOverview, RepoSessionsResponse } from '@/lib/api/types';
import { ACCOUNT_NODE, gridPositions, REPO_NODE } from './layout';
import { levelKey } from './level';
import type { CanvasLevel } from './level';
import { AccountNode, RepoDetailNode, RepoNode } from './nodes';
import type { CanvasNode } from './nodes';

// Registered once at module scope — React Flow warns when this object
// identity changes between renders.
const nodeTypes = {
  account: AccountNode,
  repo: RepoNode,
  repoDetail: RepoDetailNode,
};

/** Build the node set for the current level. Pure — exported for tests. */
export function buildNodes(args: {
  level: CanvasLevel;
  accounts: AccountOverview[];
  repos: RepoOverview[];
  repoSessions: RepoSessionsResponse | null;
  repoInstalled: boolean;
  onOpenAccount: (login: string) => void;
  onOpenRepo: (owner: string, name: string) => void;
}): CanvasNode[] {
  const { level, accounts, repos, repoSessions, repoInstalled, onOpenAccount, onOpenRepo } = args;
  switch (level.kind) {
    case 'root': {
      const positions = gridPositions(accounts.length, ACCOUNT_NODE);
      return accounts.map((account, i) => ({
        id: `account:${account.login}`,
        type: 'account' as const,
        position: positions[i]!,
        // Explicit geometry marks the node pre-measured: it renders visible
        // immediately (no ResizeObserver round-trip) and fitView can compute
        // bounds on the first frame.
        width: ACCOUNT_NODE.width,
        height: ACCOUNT_NODE.height,
        draggable: false,
        data: { account, onOpen: onOpenAccount },
      }));
    }
    case 'account': {
      const positions = gridPositions(repos.length, REPO_NODE);
      return repos.map((repo, i) => ({
        id: `repo:${repo.owner}/${repo.name}`,
        type: 'repo' as const,
        position: positions[i]!,
        width: REPO_NODE.width,
        height: REPO_NODE.height,
        draggable: false,
        data: { repo, onOpen: onOpenRepo },
      }));
    }
    case 'repo':
      return [
        {
          id: `detail:${level.owner}/${level.name}`,
          type: 'repoDetail' as const,
          position: { x: 0, y: 0 },
          draggable: false,
          data: {
            owner: level.owner,
            name: level.name,
            installed: repoInstalled,
            sessions: repoSessions,
          },
        },
      ];
  }
}

/** Runs inside <ReactFlow> (which provides the store): animate the viewport
 *  to the current node set whenever the level or node population changes.
 *  The first fit is instant; every later one is the level transition. */
function FitViewController({ dep }: { dep: string }) {
  const { fitView } = useReactFlow();
  const first = useRef(true);
  useEffect(() => {
    const duration = first.current ? 0 : 500;
    first.current = false;
    // Wait a frame so the freshly swapped nodes are measured before fitting.
    const frame = requestAnimationFrame(() => {
      void fitView({ padding: 0.16, duration, maxZoom: 1.1 });
    });
    return () => cancelAnimationFrame(frame);
  }, [dep, fitView]);
  return null;
}

/** The zoomable canvas: accounts at level 0, one account's repositories at
 *  level 1, the opened repository at level 2. Every node body is a native
 *  <button>, so the canvas is keyboard navigable without extra wiring. */
export function CanvasFlow({
  level,
  accounts,
  repos,
  repoSessions,
  repoInstalled,
  onOpenAccount,
  onOpenRepo,
}: {
  level: CanvasLevel;
  /** Already name-filtered account set (level 0). */
  accounts: AccountOverview[];
  /** Already name-filtered repo set of the selected account (level 1). */
  repos: RepoOverview[];
  /** Level-2 payload; null while loading. */
  repoSessions: RepoSessionsResponse | null;
  repoInstalled: boolean;
  onOpenAccount: (login: string) => void;
  onOpenRepo: (owner: string, name: string) => void;
}) {
  const cc = useContent().dashboard.canvas;

  const nodes = useMemo(
    () =>
      buildNodes({
        level,
        accounts,
        repos,
        repoSessions,
        repoInstalled,
        onOpenAccount,
        onOpenRepo,
      }),
    [level, accounts, repos, repoSessions, repoInstalled, onOpenAccount, onOpenRepo]
  );

  // Refit when the level changes or the visible population changes size
  // (filtering); NOT on every data refresh, which would fight the user's pan.
  const fitDep = `${levelKey(level)}:${nodes.length}`;

  return (
    <ReactFlow
      aria-label={cc.canvasAria}
      nodes={nodes}
      edges={[]}
      nodeTypes={nodeTypes}
      nodesConnectable={false}
      nodesDraggable={false}
      elementsSelectable
      minZoom={0.25}
      maxZoom={1.5}
      fitView
      proOptions={{ hideAttribution: false }}
      className="bg-bg"
    >
      <FitViewController dep={fitDep} />
    </ReactFlow>
  );
}
