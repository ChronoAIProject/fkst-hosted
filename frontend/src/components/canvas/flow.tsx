import { useEffect, useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import { Controls, ReactFlow, useReactFlow } from '@xyflow/react';
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

// Enter cue for grid nodes (accounts/repos). A node keeps the same id → the
// same DOM element across data-poll rebuilds, so the CSS animation replays
// ONLY when a node truly mounts — i.e. a filter/level change brings it in —
// giving filtered-in results a fade instead of a teleport. Opacity-only on
// purpose: `.react-flow__node` already carries React Flow's positioning
// `transform`, so a transform-based keyframe would fight it and snap the node
// to (0,0) mid-animation. `anim-overlay-in` is a pure opacity fade and is
// disabled under prefers-reduced-motion (collapses to the visible end state).
const NODE_ENTER_CLASS = 'anim-overlay-in';

// Dark/light theme tokens for the zoom/fit Controls, wired through React
// Flow's documented CSS custom properties so the buttons match the app
// surface instead of the library's default white chrome. Cast because
// custom properties are not part of the CSSProperties type.
const CONTROLS_STYLE = {
  '--xy-controls-button-background-color': 'var(--raise)',
  '--xy-controls-button-background-color-hover': 'var(--raise-2)',
  '--xy-controls-button-color': 'var(--fg)',
  '--xy-controls-button-color-hover': 'var(--fg)',
  '--xy-controls-button-border-color': 'var(--line)',
} as CSSProperties;

// Match the viewport fit the controller uses, so the Controls "fit view"
// reset re-centers to the same framing as an automatic refit.
const FIT_VIEW_OPTIONS = { padding: 0.16, maxZoom: 1.1 } as const;

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
        className: NODE_ENTER_CLASS,
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
        className: NODE_ENTER_CLASS,
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
      // Never trap the mouse wheel: a wheel event must scroll the surrounding
      // page/<main>, not zoom or pan the canvas. zoomOnScroll/panOnScroll off
      // make zoom + pan EXPLICIT gestures (drag to pan, pinch or the Controls
      // to zoom); preventScrolling=false stops React Flow from calling
      // preventDefault on the wheel, so the event reaches the page scroller.
      zoomOnScroll={false}
      panOnScroll={false}
      preventScrolling={false}
      proOptions={{ hideAttribution: false }}
      className="bg-bg"
    >
      {/* Discoverable re-center path: when a large fleet is squeezed by fitView
          against minZoom, the fit-view button restores framing and zoom in/out
          give explicit zoom now that the wheel no longer zooms. showInteractive
          is off — the lock toggles node dragging, which is disabled here. */}
      <Controls
        showInteractive={false}
        fitViewOptions={FIT_VIEW_OPTIONS}
        style={CONTROLS_STYLE}
      />
      <FitViewController dep={fitDep} />
    </ReactFlow>
  );
}
