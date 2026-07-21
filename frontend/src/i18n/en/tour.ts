import type { TourSlice } from '../slices';

// Guided product-tour copy: the `?` launcher label, the coachmark controls, and
// one card per step. Its own top-level `tour` domain, composed into the catalog
// by `en.ts`. Keys mirror the step ids in `components/tour/tour-steps.ts`.
export const tour: TourSlice = {
  helpAria: 'Take the product tour',
  closeAria: 'End the tour',
  progress: '{n} / {m}',
  skip: 'Skip',
  back: 'Back',
  next: 'Next',
  done: 'Done',
  getStarted: 'Open Get Started',
  steps: {
    welcome: {
      title: 'Welcome to fkst',
      body: 'Sessions run as GitHub-driven substrate agents that open pull requests for you. This dashboard is where you observe and control them — take a 60-second tour of what it can do.',
    },
    canvas: {
      title: 'The canvas',
      body: 'A zoomable graph of your work: click an account, then a repository, then a session to drill in. The wheel scrolls the page, drag to pan, and the Controls zoom the graph.',
    },
    breadcrumb: {
      title: 'Where you are',
      body: 'The breadcrumb shows your current level. Click any crumb — or press Esc — to step back up to a repository, an account, or the root.',
    },
    sidebar: {
      title: 'The details panel',
      body: 'This level-aware panel lists your accounts, repositories, or sessions, with the activity charts and a status legend that decodes every badge.',
    },
    newSession: {
      title: 'Start a session',
      body: 'Launch a session by creating a trigger issue: give it a name, the packages to load, a work label, an optional environment, and whether to auto-merge its pull requests.',
    },
    sessionDetail: {
      title: 'Session details',
      body: 'Open any session for four tabs — Status (lifecycle + live engine), Packages (config + queues), Logs (an in-app viewer with search), and Outcomes (pull requests with file previews).',
    },
    workItem: {
      title: 'Queue more work',
      body: 'Hand a running session another task without leaving for GitHub — it is added to the session’s work queue and picked up on the next sweep.',
    },
    environments: {
      title: 'Environments',
      body: 'Build reusable profiles of install commands, variables, and secrets here, then reference one by name when you start a session.',
    },
    newRepo: {
      title: 'Create a repository',
      body: 'Spin up a fresh repository to run sessions in — then install the App on it and open a trigger issue.',
    },
    refresh: {
      title: 'Stay up to date',
      body: 'Data refreshes on its own while you watch. Refresh forces an immediate update, and the panel shows how fresh the current view is.',
    },
    help: {
      title: 'Re-open this tour',
      body: 'Need it again? Re-launch this tour any time from the ? button. The topbar also carries a GitHub link and the language toggle; Get Started is reachable from the home page.',
    },
    finish: {
      title: 'You’re all set',
      body: 'That’s the whole dashboard. Head to Get Started for the full walkthrough, or dive straight in.',
    },
  },
};
