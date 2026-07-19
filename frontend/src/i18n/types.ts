import type { FieldKey, GrammarKey, SignalKey, StepId, FlowKey, MentalKey } from './literals';
import type { SessionHealth, SessionPhase, WorkItemState } from '@/lib/api/derive';

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
    /** Authenticated-only topbar entry opening the environments manager. */
    environments: string;
    /** Accessible name for the responsive overflow (hamburger) menu button. */
    menuAria: string;
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
  /** App-shell error states shared across every route: the render-error
   *  fallback (route errorElement + top-level ErrorBoundary), the real 404
   *  view, and the one string the global Toaster needs. */
  shell: {
    /** Render-error fallback. */
    errorTitle: string;
    errorBody: string;
    errorReload: string;
    /** Collapsible technical detail below the friendly copy. */
    errorDetailsSummary: string;
    /** 404 view. */
    notFoundEyebrow: string;
    notFoundTitle: string;
    /** `{path}` placeholder — the unmatched URL path. */
    notFoundBody: string;
    notFoundHome: string;
    notFoundMetaTitle: string;
    /** Accessible label for the global Toaster's per-notice dismiss control. */
    toastDismiss: string;
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
    /** Generic OAuth-callback error (fallback when the slug is unrecognized). */
    authError: string;
    /** Known OAuth-callback error slugs → specific copy; `authError` is the
     *  fallback for any slug not listed here. */
    authErrorBySlug: Record<string, string>;
    /** Retry action on the in-panel overview load error. */
    retry: string;
    /** Involuntary-expiry re-authenticate prompt: shown in place of the cold
     *  sign-in card so the user's level/selection is preserved. */
    sessionExpiredTitle: string;
    sessionExpiredBody: string;
    sessionExpiredAction: string;
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
      /** Poll cadence, now surfaced as the freshness line's tooltip. */
      pollNote: string;
      /** Live freshness line replacing the static poll note; `{time}` holds a
       *  relative "2 min ago" rendered by the formatter. */
      sessionsFreshness: string;
      /** Retry affordance on the no-data sessions load error. */
      sessionsRetry: string;
      /** Spinner label while a manual sessions refresh is in flight. */
      sessionsRefreshing: string;
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
      /** First-run empty state (level 0): heading shown when the viewer has
       *  connected the App to no account yet. */
      firstRunTitle: string;
      /** One-line explanation under the first-run heading. */
      firstRunBody: string;
      /** Primary "Install the GitHub App" call-to-action in the first-run state. */
      firstRunInstall: string;
      /** Secondary link from the first-run state to the get-started guide. */
      firstRunGuide: string;
      /** Persistent badge on a level-1 repo row that still needs the App. */
      needsInstall: string;
      /** Tooltip on the needs-install badge. */
      needsInstallHint: string;
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
      createOutputLangLabel: string;
      /** Hint naming what the value drives (the '### Output Language' section). */
      createOutputLangHint: string;
      createSubmit: string;
      createPending: string;
      createFailed: string;
      /** Modals-cluster additions for the create-trigger environment picker +
       *  work-label hint (owned by the create-trigger modal item). */
      createEnvironmentNone: string;
      createEnvironmentNote: string;
      createEnvironmentLoadFailed: string;
      createWorkLabelHint: string;
      createWorkLabelHintLink: string;
      createdToast: string;
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
      /** Success toast raised when a repository is created. */
      createdToast: string;
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
    /** The per-session detail drawer (status / packages / logs / outcomes). */
    detail: {
      /** "Details" action on a session card + its aria label (`{name}`). */
      open: string;
      openAria: string;
      /** Drawer heading + close affordance. */
      title: string;
      close: string;
      closeAria: string;
      tabsAria: string;
      tabStatus: string;
      tabPackages: string;
      tabLogs: string;
      tabOutcomes: string;
      /** Copy-affordance label on the full session id in the drawer header. */
      sessionIdCopy: string;
      // ---- Status tab ----
      lifecycle: string;
      /** Decoded lifecycle phase labels — one per `SessionPhase`. */
      phase: Record<SessionPhase, string>;
      healthLabel: string;
      /** Decoded health labels — one per `SessionHealth`. */
      health: Record<SessionHealth, string>;
      workItems: string;
      noWorkItems: string;
      /** Decoded work-item state labels — one per `WorkItemState`. */
      work: Record<WorkItemState, string>;
      liveEngine: string;
      liveEngineLoading: string;
      /** Note that observe is a slow pod exec. */
      liveEngineSlow: string;
      liveEngineError: string;
      liveEngineEmpty: string;
      queues: string;
      queueDepth: string;
      queuePending: string;
      queueInFlight: string;
      queueRetrying: string;
      /** Pending-delivery count line; `{n}` placeholder. */
      deliveries: string;
      /** Dead-letter count line; `{n}` placeholder. */
      deadLetters: string;
      // ---- Packages tab ----
      packagesNone: string;
      packageRefAria: string;
      /** Copy-affordance label on each package `<code>` ref. */
      packageRefCopy: string;
      queueActivity: string;
      // ---- Packages tab: frozen-config panel ----
      configLabel: string;
      /** Note that the config below is frozen at registration. */
      configFrozenNote: string;
      configWorkLabel: string;
      configEnvironment: string;
      configAutoMerge: string;
      configOutputLang: string;
      configLogAccess: string;
      /** Placeholder for a scalar the session did not carry. */
      configUnset: string;
      configYes: string;
      configNo: string;
      /** Rendered when the log-access allowlist is empty. */
      configLogAccessNone: string;
      // ---- Logs tab ----
      logsUnavailable: string;
      logsLoading: string;
      logsError: string;
      logsEmpty: string;
      logsFilesAria: string;
      logsSelectFile: string;
      logsFileLoading: string;
      logsFileError: string;
      logsSearchPlaceholder: string;
      /** Match count for the in-file find; `{n}` placeholder. */
      logsSearchCount: string;
      logsRefresh: string;
      /** Retry affordance on a failed manifest load. */
      logsRetry: string;
      /** Tail notice; `{shown}` / `{total}` placeholders (already formatted). */
      logsTruncated: string;
      /** Match count when the shown content is only a tail; `{n}` placeholder.
       *  Nudges the reader to load the full file to search everything. */
      logsSearchCountTail: string;
      /** Action that fetches the whole file (drops the tail window). */
      logsLoadFull: string;
      /** In-flight label while the full file loads. */
      logsLoadingFull: string;
      /** Shown when a failed Refresh left the last-good content on screen. */
      logsStale: string;
      /** Copy-affordance label on the shown log file's name. */
      logsFilenameCopy: string;
      logsDownloadBundle: string;
      // ---- Outcomes tab ----
      outcomesLoading: string;
      outcomesError: string;
      outcomesEmpty: string;
      outcomesFilesError: string;
      outcomesNoFiles: string;
      /** File status chip labels — known GitHub statuses; unknown falls back. */
      fileStatus: Record<'added' | 'modified' | 'removed' | 'renamed', string>;
      /** `{n}` placeholder. */
      additionsAria: string;
      /** `{n}` placeholder. */
      deletionsAria: string;
      /** `{from}` placeholder. */
      renamedFrom: string;
      /** Per-file changed-line count shown next to the row. `{n}` placeholder. */
      sizeLines: string;
      preview: string;
      previewClose: string;
      /** Explicit "fetch and preview this file's bytes" affordance — the fetch
       *  is deferred behind this click so a row never streams media blind. */
      previewLoad: string;
      previewLoading: string;
      previewError: string;
      previewTooLarge: string;
      previewBinary: string;
      openOnGithub: string;
      download: string;
      /** `{name}` placeholder. */
      downloadAria: string;
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
