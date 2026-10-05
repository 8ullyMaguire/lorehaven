/**
 * Test fixtures for the session store.
 *
 * Nine route components render a sign-in note instead of fetching while nobody is
 * signed in, and their unit tests render them directly — so each one has to say who
 * the reader is before it asserts anything. That sentence was about to be copied
 * nine times, and `Library.test.ts` already has a copy, and the copies drift:
 * `AdminMediaHealth.test.ts` writes a `me` with `id` at the top level where the type
 * has `account.id`, which passes only because the fixture is cast `as any`.
 *
 * So it lives here, in one place, in the shape `MeResponse` actually declares.
 */

import { session } from '../session.svelte';
import type { MeResponse } from '../api';

const ACCOUNT = {
  id: 'acc-1',
  email: 'reader@example.com',
  age_state: 'declared_adult' as const,
  email_verified: true,
  session_expires_at: '2026-10-01T00:00:00Z',
};

const PSEUDS = [{ id: 'pseud-1', handle: 'reader', display_name: 'Reader', bio: null }];

const CAPABILITIES = {
  can_read: true,
  can_write: true,
  can_message: true,
  can_be_listed: true,
  max_rating: 'explicit' as const,
};

/** A complete `MeResponse` for a reader who has one pseud and is signed in. */
export const SIGNED_IN_ME: MeResponse = {
  account: ACCOUNT,
  pseuds: PSEUDS,
  active_pseud_id: 'pseud-1',
  capabilities: CAPABILITIES,
  // A new account, not a curator. Several pages read a trust level before they
  // fetch anything, and a fixture claiming curator here would let a test pass
  // against a view a plain reader cannot see.
  trust_level: 1,
};

/**
 * Make `session` say a reader is signed in.
 *
 * Call from a test's `beforeEach`, or from the test itself when the signed-out case
 * is the one under test — `signOut()` is the same operation, so there is one way to
 * do it and it is the one that resets both halves of the store. A test that sets
 * `status` alone leaves a stale `me` behind, and the next component to read
 * `activePseud` gets the previous test's identity.
 */
export function signInTestReader(me: MeResponse = SIGNED_IN_ME): void {
  session.status = 'signed-in';
  session.me = me;
  session.error = null;
}

/** Make `session` say nobody is signed in — the state nine pages now render a note for. */
export function signOutTestReader(): void {
  session.status = 'anonymous';
  session.me = null;
  session.error = null;
}

/**
 * Put the store in its boot state: nobody has answered yet.
 *
 * `SessionStore` starts here — `status = 'unknown'` — and `App.svelte` resolves it
 * with one `refresh()`. A component that has to test the gate's third branch
 * (wait, rather than guess signed-in or signed-out) needs a way to say so that does
 * not involve reaching into a private field or an `as any` cast. `unknown` is
 * reachable in the app, so it is reachable in a test.
 */
export function bootTestSession(): void {
  session.status = 'unknown';
  session.me = null;
  session.error = null;
}

/**
 * Put the store back where it started.
 *
 * `SessionStore` is a module singleton, so a test file that signs a reader in leaks
 * that identity into every file vitest runs afterwards in the same worker. Without
 * this, a component test that renders nothing conditional on the session passes for
 * the wrong reason once a neighbour has signed a reader in — which is the same
 * shape as the defect these tests were written to prevent.
 */
export function resetTestSession(): void {
  signOutTestReader();
}
