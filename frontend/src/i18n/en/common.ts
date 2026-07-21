import type { CommonSlice } from '../slices';

// Shared chrome present on every page: language toggle, auth actions, footer.
export const common: CommonSlice = {
  toggle: {
    aria: 'Language',
    en: 'EN',
    zh: '中文',
  },
  auth: {
    signIn: 'Sign in with GitHub',
    signOut: 'Sign out',
  },
  footer: {
    tagline: '· ChronoAI hosted cloud',
    github: 'GitHub',
    manual: 'Operator manual',
  },
  shell: {
    errorTitle: 'Something went wrong',
    errorBody:
      'An unexpected error interrupted this page. Reloading usually clears it — if it keeps happening, the detail below helps track it down.',
    errorReload: 'Reload the page',
    errorDetailsSummary: 'Error details',
    notFoundEyebrow: 'Error 404',
    notFoundTitle: 'This page does not exist',
    notFoundBody:
      'Nothing is routed at `{path}`. It may have moved, or the link was mistyped.',
    notFoundHome: 'Back to home →',
    notFoundMetaTitle: 'FKST — Page not found',
    toastDismiss: 'Dismiss',
  },
};
