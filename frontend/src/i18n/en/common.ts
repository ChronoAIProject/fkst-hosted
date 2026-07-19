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
    getStarted: 'Get Started',
    github: 'GitHub',
    manual: 'Operator manual',
  },
};
