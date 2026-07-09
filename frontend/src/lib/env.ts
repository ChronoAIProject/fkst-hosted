/**
 * Backend API base URL for the (optional) authenticated dashboard surface, e.g.
 * `https://api.hosted.chronoai.co`. Empty string = same-origin. Set at build time
 * via `VITE_FKST_API_BASE`. The static docs pages never use this; only the
 * login/dashboard features do, and they degrade gracefully when it is unset.
 */
export const API_BASE = (import.meta.env.VITE_FKST_API_BASE ?? '').replace(/\/$/, '');

/** Whether the backend base URL is configured (login/dashboard can be attempted). */
export const API_CONFIGURED = API_BASE.length > 0;
