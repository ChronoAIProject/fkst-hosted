declare global {
  interface ImportMetaEnv {
    readonly VITE_FKST_API_BASE?: string;
    readonly VITE_NYXID_BASE?: string;
  }

  interface ImportMeta {
    readonly env: ImportMetaEnv;
  }
}

/**
 * Returns the base URL for the FKST API.
 * If VITE_FKST_API_BASE is not set, it defaults to an empty string,
 * which results in same-origin requests.
 */
export function apiBase(): string {
  return import.meta.env.VITE_FKST_API_BASE ?? '';
}

/**
 * Returns the base URL for the NyxID IAM service.
 * Reserved for future authentication and identity integrations.
 */
export function nyxidBase(): string {
  return import.meta.env.VITE_NYXID_BASE ?? '';
}
