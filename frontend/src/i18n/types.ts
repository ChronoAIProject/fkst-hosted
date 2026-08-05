import type { ChatContent } from './chat-types';
import type { DashboardContent } from './dashboard-types';
import type { OperationsContent } from './operations-types';
import type { GetStartedContent, IntroContent } from './pages-types';
import type { TitleBody } from './shared-types';
import type { WorkflowsContent } from './workflows-types';

export type { Lang, LifecycleCard, TitleBody } from './shared-types';

/**
 * The full translatable surface of the site. Both `en` and `zh` implement this
 * shape exactly. Strings may carry lightweight inline markup rendered by
 * `<Rich>`: `` `code` `` → mono chip, `**bold**` → emphasis, `*italic*` → em.
 * Any GitHub identifier / command / regex inside a string is written with
 * backticks and MUST be copied verbatim across languages.
 *
 * Each large domain lives in its own `*-types.ts` module and is referenced here.
 * That keeps every file under the 500-line limit AND keeps this one readable as
 * what it is: the index of the site's translatable surface. `slices.ts` still
 * derives every per-module authoring alias from this interface, so the split is
 * invisible to the catalogs.
 */
export interface SiteContent {
  nav: {
    home: string;
    dashboard: string;
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
  dashboard: DashboardContent;
  intro: IntroContent;
  gs: GetStartedContent;
  chat: ChatContent;
  /** Guided product tour: the `?` help launcher, the coachmark controls shared
   *  by every step, and one `{title, body}` card per step. The `steps` keys map
   *  1:1 to the step ids in `components/tour/tour-steps.ts`. */
  tour: {
    /** Accessible name for the topbar `?` launcher. */
    helpAria: string;
    /** Accessible name for the coachmark/modal end-tour control. */
    closeAria: string;
    /** Step counter; `{n}` = current (1-based), `{m}` = total. */
    progress: string;
    skip: string;
    back: string;
    next: string;
    done: string;
    /** Finish-step CTA that navigates to the Get Started route. */
    getStarted: string;
    steps: {
      welcome: TitleBody;
      canvas: TitleBody;
      breadcrumb: TitleBody;
      sidebar: TitleBody;
      newSession: TitleBody;
      sessionDetail: TitleBody;
      workItem: TitleBody;
      environments: TitleBody;
      newRepo: TitleBody;
      refresh: TitleBody;
      help: TitleBody;
      finish: TitleBody;
    };
  };
  operations: OperationsContent;
  workflows: WorkflowsContent;
}
