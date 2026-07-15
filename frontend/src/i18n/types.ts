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
      /** Group label for the viewer's own account. */
      personalGroup: string;
      /** Group label for an organization account. */
      orgGroup: string;
      /** Per-group counts template; `{installed}` / `{total}` placeholders. */
      groupCounts: string;
      /** Body line for a group (org creation target) with no repositories. */
      groupEmpty: string;
      searchPlaceholder: string;
      /** One-line empty state when a search matches nothing. */
      searchEmpty: string;
      /** "New repository" button in the section header. */
      newRepo: string;
      createTitle: string;
      ownerLabel: string;
      /** Personal option in the owner select; `{login}` placeholder. */
      ownerPersonal: string;
      nameLabel: string;
      /** Hint under the name input (allowed characters). */
      nameHint: string;
      privateLabel: string;
      descriptionLabel: string;
      submit: string;
      creating: string;
      cancel: string;
      /** Generic creation failure (no server message available). */
      createFailed: string;
      /** Callout under a freshly created repo pointing at the Install step. */
      createdNextStep: string;
      /** Group-header CTA for an account without an App installation. */
      connect: string;
      /** Short hint next to the Connect CTA (why connecting matters). */
      connectHint: string;
      /** Group-header link to the installation's GitHub settings page. */
      manage: string;
      /** Group-header danger action for a connected account. */
      uninstall: string;
      /** Uninstall confirm dialog title; `{owner}` placeholder. */
      uninstallConfirmTitle: string;
      /** Uninstall confirm dialog body; `{owner}` placeholder. */
      uninstallConfirmBody: string;
      /** Uninstall confirm dialog: explicit confirm button. */
      uninstallConfirm: string;
      /** Uninstall confirm button while the DELETE is in flight. */
      uninstallPending: string;
      /** Generic uninstall failure (no server message available). */
      uninstallFailed: string;
      /** Per-repo action removing it from a selected-mode installation. */
      remove: string;
      /** Remove confirm dialog title; `{repo}` placeholder. */
      removeConfirmTitle: string;
      /** Remove confirm dialog body; `{repo}` placeholder. */
      removeConfirmBody: string;
      /** Remove confirm dialog: explicit confirm button. */
      removeConfirm: string;
      /** Remove confirm button while the DELETE is in flight. */
      removePending: string;
      /** Generic remove failure (no server message available). */
      removeFailed: string;
      /** Title hint on Installed rows of an all-repositories installation. */
      allModeHint: string;
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
