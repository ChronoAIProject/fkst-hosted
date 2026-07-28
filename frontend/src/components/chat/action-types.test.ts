import { describe, it, expect } from 'vitest';
import { mapDraftToRequest, parseActionProposal } from './action-types';
import type { DraftSessionRequest } from './action-types';

const target = { method: 'POST', path: '/api/v1/repos/acme/site/sessions' };

const draft = (over: Partial<DraftSessionRequest> = {}): DraftSessionRequest => ({
  name: 'sitebuilder',
  packages: ['acme/pkgs@main:packages/site'],
  manifests: [],
  work_label: 'site-build',
  environment: null,
  source_branch: null,
  target_branch: null,
  auto_merge: null,
  log_access: [],
  collaborators: [],
  output_lang: null,
  ...over,
});

const sessionProposal = (over: Record<string, unknown> = {}) => ({
  kind: 'create_session',
  owner: 'acme',
  name: 'site',
  request: draft(),
  rendered_issue_body: '### Session Name\n\nsitebuilder\n',
  summary: 'Start session `sitebuilder`',
  target,
  ...over,
});

const workItemProposal = (over: Record<string, unknown> = {}) => ({
  kind: 'create_work_item',
  owner: 'acme',
  name: 'site',
  trigger_issue_number: 7,
  title: 'Add the footer',
  label: 'site-build',
  body: 'Edit src/footer.tsx',
  summary: 'Queue a work item',
  target,
  ...over,
});

const stopProposal = (over: Record<string, unknown> = {}) => ({
  kind: 'stop_session',
  owner: 'acme',
  name: 'site',
  trigger_issue_number: 7,
  reason: 'the work is finished',
  summary: 'Stop the session',
  target: { method: 'DELETE', path: '/api/v1/repos/acme/site/sessions/7' },
  ...over,
});

describe('parseActionProposal — valid payloads', () => {
  it('accepts a create-session proposal', () => {
    const parsed = parseActionProposal(sessionProposal());
    expect(parsed?.kind).toBe('create_session');
  });

  it('accepts a work-item proposal', () => {
    expect(parseActionProposal(workItemProposal())?.kind).toBe('create_work_item');
  });

  it('accepts a work-item proposal with no label and an empty body', () => {
    // Both are legitimately optional: the endpoint falls back to the trigger's own
    // work label, and a body-less issue is allowed.
    expect(parseActionProposal(workItemProposal({ label: null, body: '' }))?.kind).toBe(
      'create_work_item'
    );
  });

  it('accepts a stop proposal', () => {
    expect(parseActionProposal(stopProposal())?.kind).toBe('stop_session');
  });

  it('accepts a draft whose optional fields are absent rather than null', () => {
    const sparse = {
      name: 'x',
      packages: ['a/b@c:d'],
      manifests: [],
      log_access: [],
      collaborators: [],
    };
    expect(parseActionProposal(sessionProposal({ request: sparse }))?.kind).toBe('create_session');
  });
});

describe('parseActionProposal — rejections', () => {
  it('rejects a non-object', () => {
    for (const value of [null, undefined, 'x', 7, []]) {
      expect(parseActionProposal(value)).toBeNull();
    }
  });

  it('rejects an unrecognized kind', () => {
    // The union has exactly three variants; there is deliberately no
    // "unsupported action" card, because one the SPA cannot execute is worse than
    // an honest note.
    expect(parseActionProposal(sessionProposal({ kind: 'delete_repository' }))).toBeNull();
    expect(parseActionProposal(sessionProposal({ kind: undefined }))).toBeNull();
  });

  it('rejects a missing owner, name or summary', () => {
    expect(parseActionProposal(sessionProposal({ owner: '' }))).toBeNull();
    expect(parseActionProposal(sessionProposal({ name: '  ' }))).toBeNull();
    expect(parseActionProposal(sessionProposal({ summary: undefined }))).toBeNull();
  });

  it('rejects a malformed target', () => {
    expect(parseActionProposal(sessionProposal({ target: undefined }))).toBeNull();
    expect(parseActionProposal(sessionProposal({ target: { method: 'POST' } }))).toBeNull();
  });

  it('rejects a create-session proposal with no rendered body', () => {
    // The rendered body IS the preview; without it the confirm gate shows nothing.
    expect(parseActionProposal(sessionProposal({ rendered_issue_body: undefined }))).toBeNull();
  });

  it('rejects a malformed draft', () => {
    expect(parseActionProposal(sessionProposal({ request: undefined }))).toBeNull();
    expect(parseActionProposal(sessionProposal({ request: draft({ name: '' }) }))).toBeNull();
    // Array fields must really be arrays of strings.
    expect(
      parseActionProposal(sessionProposal({ request: { ...draft(), packages: [7] } }))
    ).toBeNull();
    expect(
      parseActionProposal(sessionProposal({ request: { ...draft(), auto_merge: 'yes' } }))
    ).toBeNull();
  });

  it('rejects a bad issue number', () => {
    for (const value of [0, -1, 1.5, '7', undefined]) {
      expect(parseActionProposal(workItemProposal({ trigger_issue_number: value }))).toBeNull();
      expect(parseActionProposal(stopProposal({ trigger_issue_number: value }))).toBeNull();
    }
  });

  it('rejects a work item with no title', () => {
    expect(parseActionProposal(workItemProposal({ title: '   ' }))).toBeNull();
  });

  it('rejects a stop proposal with no reason', () => {
    // Stopping is irreversible; the user must see why it is being suggested.
    expect(parseActionProposal(stopProposal({ reason: '' }))).toBeNull();
  });
});

describe('mapDraftToRequest', () => {
  it('carries every provided field through', () => {
    const request = mapDraftToRequest(
      draft({
        environment: 'my-env',
        source_branch: 'main',
        target_branch: 'integration',
        auto_merge: true,
        log_access: ['alice'],
        collaborators: ['carol'],
        output_lang: 'en',
      })
    );
    expect(request).toMatchObject({
      name: 'sitebuilder',
      packages: ['acme/pkgs@main:packages/site'],
      work_label: 'site-build',
      environment: 'my-env',
      source_branch: 'main',
      target_branch: 'integration',
      auto_merge: true,
      log_access: ['alice'],
      collaborators: ['carol'],
      output_lang: 'en',
    });
  });

  it('turns nulls into omissions rather than explicit nulls', () => {
    const request = mapDraftToRequest(draft({ work_label: null, auto_merge: null }));
    expect(request.work_label).toBeUndefined();
    expect(request.auto_merge).toBeUndefined();
  });

  it('never produces a disposable-environment field', () => {
    // The draft type has no field for secrets, so none can appear on the wire —
    // which is exactly why a dedicated DTO exists.
    const request = mapDraftToRequest(draft()) as unknown as Record<string, unknown>;
    expect(request.disposable_environment).toBeUndefined();
    expect(Object.keys(request)).not.toContain('disposable_environment');
  });
});
