// The primitives every content module shares.
//
// They live apart from `types.ts` so a domain module can name them without
// importing the composed `SiteContent` back from the file that imports it.

export type Lang = 'en' | 'zh';

/** A two-line card: a heading and one paragraph. */
export interface TitleBody {
  title: string;
  body: string;
}

/** One step of the session lifecycle strip: a term and its definition. */
export interface LifecycleCard {
  t: string;
  d: string;
}
