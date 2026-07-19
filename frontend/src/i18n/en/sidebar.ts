import type { CanvasSidebarSlice } from '../slices';

// The canvas "details panel" (right-hand sidebar): the per-level ledes and the
// sessions / pull-requests listing shown for the selected node. Composed into
// `dashboard.canvas` by index. Owned by the sidebar cluster.
export const canvasSidebar: CanvasSidebarSlice = {
  sidebarAria: 'Details panel',
  loadingSidebar: 'Loading details…',
  viewRoot:
    'You are looking at every GitHub account you can reach — your personal account and your organizations. The dots inside each card are its repositories, carrying the same status colors. Click an account to zoom in.',
  viewAccount:
    'You are looking at the repositories of {login}. Click a repository to open its fkst sessions.',
  viewRepo:
    'You are looking at the fkst sessions of {repo} — every trigger issue with its work issues and pull requests.',
  sessionsTitle: 'Sessions',
  pollNote: 'Auto-refreshes every 15 s while open.',
  sessionsFreshness: 'updated {time}',
  sessionsRetry: 'Retry',
  sessionsRefreshing: 'Refreshing…',
  sessionsLoadFailed: 'Could not load the sessions of this repository. Please try again.',
  sessionsStaleNotice: 'Refresh failed — showing the last loaded sessions.',
  notInstalledNote: 'The App is not installed on this repository, so sessions cannot run here.',
  livenessStarting: 'starting',
  livenessLive: 'live',
  livenessTerminating: 'terminating',
  logDownload: 'Download logs',
  prsTitle: 'Pull requests',
  prMerged: 'merged',
  prForIssue: 'for #{n}',
  createdWord: 'created',
  updatedWord: 'updated',
  closedWord: 'closed',
  firstRunTitle: 'Get started with fkst',
  firstRunBody:
    'Install the GitHub App on your account or an organization to let fkst run coding sessions straight from your GitHub issues.',
  firstRunInstall: 'Install the GitHub App',
  firstRunGuide: 'How it works →',
  needsInstall: 'Needs install',
  needsInstallHint: 'Install the App on this repository so its sessions can run here.',
};
