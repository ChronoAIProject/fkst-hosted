import { test, expect, type Page } from '@playwright/test';
import { installApiRoutes, seedAuth } from './fixtures';
import { createEnvStore, type EnvStore } from './env-fixtures';
import { drawerTab, openAccount, openRepo, settle, shot } from './harness';

// The named-environment CRUD "parity" surface: a user can define reusable
// install steps / variables / secrets once and then a session may only reference
// an environment that exists. These specs prove the whole round-trip against a
// stateful mock store — create, list, the inline install-validation error, the
// create-trigger picker parity, delete — plus the security invariant that secret
// VALUES never reach the UI.

/** Open the Environments drawer from the topbar (authenticated only). */
async function openEnvironments(page: Page) {
  await page.getByRole('button', { name: 'Environments' }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
}

/** Fill the editor's first install / variable / secret rows. */
async function fillEditor(
  page: Page,
  opts: { name: string; install: string; varName: string; varValue: string; secretName: string; secretValue: string }
) {
  const d = page.getByRole('dialog');
  await d.getByLabel('Name', { exact: true }).fill(opts.name);
  await d.getByLabel('Install commands 1').fill(opts.install);
  await d.getByLabel('Variables 1 NAME').fill(opts.varName);
  await d.getByLabel('Variables 1 value', { exact: true }).fill(opts.varValue);
  await d.getByLabel('Secrets 1 NAME').fill(opts.secretName);
  await d.getByLabel('Secrets 1 value (write-only)').fill(opts.secretValue);
}

test.describe('environment-profile CRUD parity', () => {
  let store: EnvStore;

  test.beforeEach(async ({ page }) => {
    store = createEnvStore();
    await seedAuth(page);
    await installApiRoutes(page, { envStore: store });
    await page.goto('/dashboard');
    await expect(page.getByRole('heading', { name: 'Your fkst sessions' })).toBeVisible();
    await settle(page);
  });

  test('create → appears in list with counts → appears in the trigger Environment select', async ({
    page,
  }) => {
    await openEnvironments(page);
    const drawer = page.getByRole('dialog');
    // The seeded environment is listed on open.
    await expect(drawer.getByText('video-studio')).toBeVisible();

    await drawer.getByRole('button', { name: 'New environment' }).click();
    await fillEditor(page, {
      name: 'web-scraper',
      install: 'pip install -r requirements.txt',
      varName: 'HEADLESS',
      varValue: 'true',
      secretName: 'API_KEY',
      secretValue: 'topsecret',
    });
    await shot(page, 'env-01-editor-filled');
    await drawer.getByRole('button', { name: 'Save', exact: true }).click();

    // Success returns to the list; the new environment shows with its counts.
    await expect(drawer.getByText('web-scraper')).toBeVisible();
    const row = drawer.getByRole('button', { name: 'Open environment web-scraper' });
    await expect(row.getByText('1 install')).toBeVisible();
    await expect(row.getByText('1 variable')).toBeVisible();
    await expect(row.getByText('1 secret')).toBeVisible();
    // Success toast fired.
    await expect(page.getByText('Environment “web-scraper” saved.')).toBeVisible();
    await shot(page, 'env-02-created-in-list');

    // Close the drawer and drive to a repo's create-session dialog: the new
    // profile must be selectable there (parity — no dangling references).
    await drawer.getByRole('button', { name: 'Close environments' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await openAccount(page, 'octo-dev');
    await openRepo(page, 'octo-dev', 'web-app');
    await page.getByRole('button', { name: 'New session' }).click();
    const modal = page.getByRole('dialog');
    await expect(modal).toBeVisible();
    const select = modal.locator('#trigger-environment');
    // selectOption throws if the option is absent — proving the profile is there.
    await select.selectOption({ label: 'web-scraper' });
    await expect(select).toHaveValue('web-scraper');
    await shot(page, 'env-03-trigger-select-parity');
  });

  test('a failed install validation (422) renders the InstallValidationError inline', async ({
    page,
  }) => {
    await openEnvironments(page);
    const drawer = page.getByRole('dialog');
    await drawer.getByRole('button', { name: 'New environment' }).click();
    // 'bad-env' is the store's designated failing name → PUT returns the 422.
    await fillEditor(page, {
      name: 'bad-env',
      install: 'pip install nonexistent-pkg==9.9.9',
      varName: 'A',
      varValue: 'b',
      secretName: 'S',
      secretValue: 'x',
    });
    await drawer.getByRole('button', { name: 'Save', exact: true }).click();

    // The detailed report renders inline; nothing was persisted (no toast, no
    // list navigation). Assert the load-bearing fields.
    await expect(drawer.getByText('Install validation failed')).toBeVisible();
    await expect(drawer.getByText('Command index')).toBeVisible();
    await expect(drawer.getByText('pip install nonexistent-pkg==9.9.9')).toBeVisible();
    await expect(drawer.getByText('Exit code')).toBeVisible();
    await expect(drawer.getByText('2', { exact: true })).toBeVisible(); // exit_code
    // stderr tail is shown verbatim.
    await expect(drawer.getByText(/No matching distribution found/)).toBeVisible();
    await shot(page, 'env-04-validation-error');
  });

  test('secret VALUES are never rendered — only key names', async ({ page }) => {
    await openEnvironments(page);
    const drawer = page.getByRole('dialog');
    // Create a profile carrying a secret.
    await drawer.getByRole('button', { name: 'New environment' }).click();
    // The secret value field is masked (type=password) so it is never shown.
    await expect(drawer.getByLabel('Secrets 1 value (write-only)')).toHaveAttribute(
      'type',
      'password'
    );
    await fillEditor(page, {
      name: 'secret-holder',
      install: 'echo hi',
      varName: 'V',
      varValue: 'v',
      secretName: 'DEPLOY_TOKEN',
      secretValue: 'do-not-leak-me',
    });
    await drawer.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(drawer.getByText('secret-holder')).toBeVisible();

    // Open its detail: the secret KEY appears, the VALUE never does.
    await drawer.getByRole('button', { name: 'Open environment secret-holder' }).click();
    await expect(drawer.getByText('DEPLOY_TOKEN')).toBeVisible();
    await expect(drawer.getByText('do-not-leak-me')).toHaveCount(0);
    // And the store never held the value (contract check on the mock).
    expect(store.profiles.get('secret-holder')?.secret_keys).toEqual(['DEPLOY_TOKEN']);
    await shot(page, 'env-05-secret-keys-only');
  });

  test('delete via the ConfirmDialog removes it from the list', async ({ page }) => {
    await openEnvironments(page);
    const drawer = page.getByRole('dialog');
    await expect(drawer.getByText('video-studio')).toBeVisible();
    await drawer.getByRole('button', { name: 'Open environment video-studio' }).click();
    await expect(drawer.getByRole('heading', { name: 'video-studio' })).toBeVisible();

    await drawer.getByRole('button', { name: 'Delete', exact: true }).click();
    const confirm = page.getByRole('dialog', { name: 'Delete environment?' });
    await expect(confirm).toBeVisible();
    await confirm.getByRole('button', { name: 'Delete', exact: true }).click();

    // Back on the list, the environment is gone, and the deletion was toasted.
    await expect(page.getByText('Environment “video-studio” deleted.')).toBeVisible();
    await expect(page.getByRole('dialog').getByText('video-studio')).toHaveCount(0);
    await expect(page.getByText('No environments yet.')).toBeVisible();
    expect(store.profiles.has('video-studio')).toBe(false);
    await shot(page, 'env-06-deleted');
  });

  test('the session drawer Config panel shows the log-access allowlist and output locale', async ({
    page,
  }) => {
    await openAccount(page, 'octo-dev');
    await openRepo(page, 'octo-dev', 'web-app');
    await page.getByRole('button', { name: 'Open details for session feature-auth' }).click();
    const dialog = page.getByTestId('session-detail');
    await expect(dialog).toBeVisible();

    await drawerTab(page, 'Packages').click();
    // ConfigPanel: the frozen log-access allowlist (extra viewers) as chips…
    await expect(dialog.getByText('Log access')).toBeVisible();
    await expect(dialog.getByText('collab-bob')).toBeVisible();
    // …and the output locale.
    await expect(dialog.getByText('Output language')).toBeVisible();
    await expect(dialog.getByText('zh', { exact: true })).toBeVisible();
    await shot(page, 'env-07-config-panel');
  });
});
