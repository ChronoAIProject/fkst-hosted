import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import type { Lang, SiteContent } from './types';
import { en } from './en';
import { zh } from './zh';

const CATALOGS: Record<Lang, SiteContent> = { en, zh };
const STORAGE_KEY = 'fkst-lang';

function detectInitial(): Lang {
  if (typeof window === 'undefined') return 'en';
  try {
    const stored = window.localStorage?.getItem(STORAGE_KEY);
    if (stored === 'en' || stored === 'zh') return stored;
  } catch {
    // localStorage may be unavailable (private mode) — fall through to detection.
  }
  const nav = window.navigator.language?.toLowerCase() ?? '';
  return nav.startsWith('zh') ? 'zh' : 'en';
}

interface LanguageContextValue {
  lang: Lang;
  setLang: (l: Lang) => void;
  content: SiteContent;
}

// Default to English so components render correctly even without a provider
// (e.g. in unit tests). The provider adds real state, persistence, <html lang>.
const LanguageContext = createContext<LanguageContextValue>({
  lang: 'en',
  setLang: () => {},
  content: en,
});

export function LanguageProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(detectInitial);

  useEffect(() => {
    document.documentElement.lang = lang;
    try {
      window.localStorage.setItem(STORAGE_KEY, lang);
    } catch {
      // localStorage may be unavailable (private mode) — ignore.
    }
  }, [lang]);

  const setLang = useCallback((l: Lang) => setLangState(l), []);

  const value = useMemo<LanguageContextValue>(
    () => ({ lang, setLang, content: CATALOGS[lang] }),
    [lang, setLang]
  );

  return <LanguageContext.Provider value={value}>{children}</LanguageContext.Provider>;
}

export function useLang() {
  return useContext(LanguageContext);
}

export function useContent(): SiteContent {
  return useContext(LanguageContext).content;
}
