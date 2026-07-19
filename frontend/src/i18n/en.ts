import type { SiteContent } from './types';
import { nav } from './en/nav';
import { common } from './en/common';
import { dashboardScalars, canvasGraph, reposBase } from './en/dashboard';
import { canvasSidebar } from './en/sidebar';
import { canvasModals, reposModals } from './en/modals';
import { canvasEnv, environmentScalar } from './en/environments';
import { detail } from './en/detail';
import { pages } from './en/pages';
import { tour } from './en/tour';

// English catalog, composed from the per-domain modules under `en/`. The split
// is the point: parallel work items each own a disjoint module, so they never
// conflict on one giant string file. The `dashboard.canvas` and
// `dashboard.repos` objects are re-assembled here from their domain slices so
// the key paths consumers read stay identical. The `: SiteContent` annotation
// is the completeness backstop — every key must be present exactly once.
export const en: SiteContent = {
  nav,
  ...common,
  dashboard: {
    ...dashboardScalars,
    ...environmentScalar,
    canvas: { ...canvasGraph, ...canvasSidebar, ...canvasModals, ...canvasEnv },
    repos: { ...reposBase, ...reposModals },
    detail,
  },
  ...pages,
  tour,
};
