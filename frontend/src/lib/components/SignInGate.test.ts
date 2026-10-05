import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import Gate from './SignInGate.test.svelte';
import {
  bootTestSession,
  resetTestSession,
  signInTestReader,
  signOutTestReader,
} from '../testing/session';

/**
 * What the gate is for, in one assertion: a page behind a session must not tell a
 * signed-out visitor that something went wrong.
 *
 * Nine pages used to open with `ErrorSummary` reading "That did not work /
 * authentication required" — a correct report of a 401 and a useless thing to open
 * a page with. The reader cannot act on "sign in" if they did not know the page
 * needed a session, and the retry button re-issues a request that fails the same
 * way. The five pages that never had the bug all guard the same way, so this
 * component is that guard, extracted.
 *
 * Rendered through `SignInGate.test.svelte` because the gate wraps a snippet, and a
 * snippet cannot be built in a test without one — the same reason
 * `FieldBinding.test.svelte` exists for the field primitives.
 */
describe('SignInGate', () => {
  beforeEach(() => {
    resetTestSession();
  });

  afterEach(() => {
    resetTestSession();
  });

  it('offers the sign-in note, naming what signing in would get them', () => {
    signOutTestReader();
    render(Gate, { purpose: 'see the works you have imported' });

    const note = screen.getByTestId('signin-note');
    expect(note.textContent).toContain('see the works you have imported');
    expect(note.textContent).toContain('Sign in');
    // The link is the whole affordance, so it has to be a link.
    expect(screen.getByRole('link', { name: 'Sign in' }).getAttribute('href')).toBe('/sign-in');
  });

  it('says nothing about failure, because nothing failed', () => {
    signOutTestReader();
    render(Gate, { purpose: 'see your queue' });

    // The absence is the point. `ErrorSummary`'s heading is the exact string the
    // 2026-10-05 sweep found on nine pages, so this is a regression test against
    // the defect rather than against a paraphrase of it.
    expect(screen.queryByText('That did not work')).toBeNull();
    // And the page's own content is not rendered either, because the reader has no
    // claim to it.
    expect(screen.queryByTestId('page-body')).toBeNull();
  });

  it('renders the page for a signed-in reader', () => {
    signInTestReader();
    render(Gate, { purpose: 'see your queue' });

    expect(screen.queryByTestId('signin-note')).toBeNull();
    expect(screen.getByTestId('page-body')).toBeTruthy();
  });

  it('waits rather than guessing while the session is still being asked about', () => {
    // `App.svelte` asks once at boot, so this window is one request long. A gate
    // that treated `unknown` as signed-out would flash the sign-in note at a
    // reader who is signed in; one that treated it as signed-in would fetch and
    // 401 — which is the defect in a different costume.
    bootTestSession();

    render(Gate, { purpose: 'see your queue' });

    expect(screen.queryByTestId('signin-note')).toBeNull();
    expect(screen.queryByText('That did not work')).toBeNull();
    // The skeleton is what shows, and it announces itself rather than sitting silent.
    expect(screen.getByRole('status')).toBeTruthy();
    expect(screen.queryByTestId('page-body')).toBeNull();
  });
});
