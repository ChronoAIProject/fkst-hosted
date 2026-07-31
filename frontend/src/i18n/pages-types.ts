import type { FieldKey, GrammarKey, SignalKey, StepId } from './literals';
import type { LifecycleCard } from './shared-types';

/** The v2 landing: a single-viewport centered hero (no scrolling sections). */
export interface IntroContent {
  metaTitle: string;
  eyebrow: string;
  /** Hero headline, two stacked lines: fg-gradient top + accent-gradient bottom. */
  heroTitleTop: string;
  heroTitleAccent: string;
  heroLede: string;
  ctaStart: string;
  ctaManual: string;
  /** The decorative flow-line under the CTAs (aria-hidden, still localized). */
  pipeTrigger: string;
  pipeSession: string;
  pipeWork: string;
}

/** The long-form Get Started reference. */
export interface GetStartedContent {
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
}
