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
    /** Route-level skeleton label while the lazy dashboard chunk downloads. */
    loading: string;
    eyebrow: string;
    title: string;
    lede: string;
    signInTitle: string;
    signInBody: string;
    notConfigured: string;
    authError: string;
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
    /** The zoomable canvas surface (levels 0–2) and its sidebar. */
    canvas: {
      canvasAria: string;
      sidebarAria: string;
      breadcrumbAria: string;
      /** Root crumb label (level 0). */
      breadcrumbRoot: string;
      back: string;
      backAria: string;
      /** Small keyboard hint next to the breadcrumb. */
      escHint: string;
      loadingCanvas: string;
      loadingSidebar: string;
      legendTitle: string;
      legendNone: string;
      legendInstalled: string;
      legendActive: string;
      /** Sidebar lede stating what level 0 represents. */
      viewRoot: string;
      /** Sidebar lede for level 1; `{login}` placeholder. */
      viewAccount: string;
      /** Sidebar lede for level 2; `{repo}` placeholder. */
      viewRepo: string;
      /** Badge on accounts where the viewer is owner/admin. */
      ownerBadge: string;
      statusNone: string;
      statusInstalled: string;
      /** Active badge; `{n}` placeholder. */
      statusActiveCount: string;
      /** `{n}` placeholder. */
      repoCount: string;
      /** Overflow marker for the in-card repo dots; `{n}` placeholder. */
      moreRepos: string;
      /** `{login}` placeholder. */
      openAccountAria: string;
      /** `{repo}` placeholder. */
      openRepoAria: string;
      /** Warning when the backend flagged counts_complete=false. */
      countsIncomplete: string;
      filterAccountsPlaceholder: string;
      filterReposPlaceholder: string;
      noAccountsMatch: string;
      noReposMatch: string;
      noAccounts: string;
      chartSessionsTitle: string;
      chartPackagesTitle: string;
      chartScopeAllAccounts: string;
      chartScopeAllRepos: string;
      chartScopeAriaAccounts: string;
      chartScopeAriaRepos: string;
      chartEmpty: string;
      /** Aggregate row label when the chart tail is folded. */
      chartOther: string;
      sessionsTitle: string;
      /** Note that level-2 data refreshes on a poll. */
      pollNote: string;
      sessionsLoadFailed: string;
      /** Non-blocking notice when a refresh failed but last-good data shows. */
      sessionsStaleNotice: string;
      notInstalledNote: string;
      newTrigger: string;
      livenessStarting: string;
      livenessLive: string;
      livenessTerminating: string;
      logDownload: string;
      prsTitle: string;
      prMerged: string;
      /** PR → work-issue link text; `{n}` placeholder. */
      prForIssue: string;
      createdWord: string;
      updatedWord: string;
      closedWord: string;
      stop: string;
      /** `{name}` placeholder. */
      stopAria: string;
      /** `{name}` placeholder. */
      stopConfirmTitle: string;
      /** `{number}` placeholder (the trigger issue number). */
      stopConfirmBody: string;
      stopConfirm: string;
      stopPending: string;
      stopFailed: string;
      createTitle: string;
      createNameLabel: string;
      createNameHint: string;
      createPackagesLabel: string;
      createPackagePlaceholder: string;
      addPackage: string;
      /** `{n}` placeholder (1-based row index). */
      removePackageAria: string;
      createWorkLabelLabel: string;
      createEnvironmentLabel: string;
      createAutoMergeLabel: string;
      createLogAccessLabel: string;
      createLogAccessHint: string;
      createSubmit: string;
      createPending: string;
      createFailed: string;
    };
    repos: {
      refresh: string;
      /** Refresh button label while the overview re-fetch is in flight. */
      refreshing: string;
      loadFailed: string;
      /** Non-blocking notice when a refresh failed but last-good data shows. */
      refreshFailedStale: string;
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
      manageRepoHint: string;
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
