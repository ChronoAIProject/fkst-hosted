import type { FieldKey, GrammarKey, SignalKey, StepId, FlowKey, MentalKey } from './literals';

export type Lang = 'en' | 'zh';

export interface TermCard {
  term: string;
  is: string;
  control: string;
}

export interface FlowStep {
  label: string;
  sub: string;
}

export interface TitleBody {
  title: string;
  body: string;
}

export interface LifecycleCard {
  t: string;
  d: string;
}

/**
 * The full translatable surface of the site. Both `en` and `zh` implement this
 * shape exactly. Strings may carry lightweight inline markup rendered by
 * `<Rich>`: `` `code` `` → mono chip, `**bold**` → emphasis, `*italic*` → em.
 * Any GitHub identifier / command / regex inside a string is written with
 * backticks and MUST be copied verbatim across languages.
 */
export interface SiteContent {
  nav: {
    introduction: string;
    getStarted: string;
    dashboard: string;
    getStartedCta: string;
    homeAria: string;
  };
  toggle: {
    aria: string;
    en: string;
    zh: string;
  };
  auth: {
    signIn: string;
    signOut: string;
  };
  footer: {
    tagline: string;
    getStarted: string;
    github: string;
    manual: string;
  };
  dashboard: {
    metaTitle: string;
    eyebrow: string;
    title: string;
    lede: string;
    signInTitle: string;
    signInBody: string;
    notConfigured: string;
    authError: string;
    update: string;
    updating: string;
    updatesNote: string;
    lastUpdated: string;
    never: string;
    updateFailed: string;
    firstVisitTitle: string;
    firstVisitBody: string;
    loadingTitle: string;
    reposScanned: string;
    noRepos: string;
    noSessions: string;
    installed: string;
    workLabel: string;
    packages: string;
    autoMerge: string;
    environment: string;
    invalidTrigger: string;
    trigger: string;
    workIssues: string;
    open: string;
    closed: string;
    repos: {
      title: string;
      refresh: string;
      loading: string;
      loadFailed: string;
      empty: string;
      private: string;
      public: string;
      org: string;
      installed: string;
      install: string;
      nonAdminHint: string;
      appNotConfigured: string;
    };
  };
  intro: {
    metaTitle: string;
    eyebrow: string;
    heroTitle: string;
    heroLede: string; // rendered right after the FKST wordmark
    ctaStart: string;
    ctaManual: string;
    whatIsEyebrow: string;
    whatIsTitle: string;
    whatIsBody: string[];
    thesis: string;
    mentalEyebrow: string;
    mental: Record<MentalKey, TermCard>;
    providesEyebrow: string;
    providesTitle: string;
    features: TitleBody[];
    flowEyebrow: string;
    flow: Record<FlowKey, FlowStep>;
    ctaTitle: string;
    ctaBody: string;
    ctaButton: string;
  };
  gs: {
    metaTitle: string;
    eyebrow: string;
    title: string;
    lede: string;
    stepWord: string;
    stepTitles: Record<StepId, string>;
    requiredLabel: string;
    optionalLabel: string;
    install: { body: string; calloutTitle: string; callout: string };
    start: {
      body: string;
      exampleCaption: string;
      createIntro: string;
      terminalCaption: string;
      calloutTitle: string;
      callout: string;
    };
    parameters: {
      intro: string;
      fieldRules: Record<FieldKey, string>;
      calloutTitle: string;
      callout: string;
    };
    packages: {
      intro: string;
      grammar: Record<GrammarKey, string>;
      exampleCaption: string;
    };
    queue: { body: string; calloutTitle: string; callout: string };
    status: {
      intro: string;
      onWord: string;
      kind: Record<SignalKey, string>;
      where: Record<SignalKey, string>;
      meaning: Record<SignalKey, string>;
    };
    logs: {
      intro: string;
      browserTitle: string;
      browser: string;
      apiTitle: string;
      api: string;
      terminalCaption: string;
      calloutTitle: string;
      callout: string;
    };
    lifecycle: LifecycleCard[];
    rulesEyebrow: string;
    rulesTitle: string;
    rules: string[];
    fullRefPrefix: string;
    fullRefLink: string;
  };
}
