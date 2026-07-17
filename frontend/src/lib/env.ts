/**
 * Backend API base URL for the authenticated login/dashboard surface. Empty
 * string (the default) = SAME-ORIGIN — the normal deployable topology where one
 * ingress fronts both this SPA and the backend.
 * Set `VITE_FKST_API_BASE` at build time only for a cross-origin backend.
 */
export const API_BASE = (import.meta.env.VITE_FKST_API_BASE ?? '').replace(/\/$/, '');

/**
 * Whether the login/dashboard features are enabled. On by default (same-origin
 * counts as configured); a standalone docs-only hosting of the static site can
 * opt out with `VITE_FKST_DOCS_ONLY=true`, which keeps the Dashboard tab in its
 * "backend not configured" state instead of firing doomed API calls.
 */
export const API_CONFIGURED = import.meta.env.VITE_FKST_DOCS_ONLY !== 'true';
