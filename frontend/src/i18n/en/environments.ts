import type { CanvasEnvSlice, EnvironmentScalarSlice } from '../slices';

// Environment strings. Today this is only the "Environment" chip on a session
// card and the environment field label in the create-session form; a fuller
// environment-management surface arrives in a later wave, which edits THIS file
// (never the sibling dashboard/sidebar/modals modules).

/** Composed into the dashboard scalars (`dashboard.environment`). */
export const environmentScalar: EnvironmentScalarSlice = {
  environment: 'Environment',
};

/** Composed into `dashboard.canvas` alongside the create-session dialog. */
export const canvasEnv: CanvasEnvSlice = {
  createEnvironmentLabel: 'Environment (optional)',
};
