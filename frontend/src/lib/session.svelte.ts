/**
 * Who is signed in, and which pseud they are speaking as.
 *
 * One store for the whole shell, because the answer has to be the same in the
 * header, on the account page and on the pseud pages; fetching `/auth/me` in
 * each place is how two parts of an interface come to disagree.
 *
 * The distinction that matters below is between "the server says we have no
 * session" and "we could not ask". The first is `anonymous`; the second leaves
 * the status `unknown` and records the failure, because a network blip is not
 * evidence that somebody was signed out.
 */

import {
  ApiError,
  type MeResponse,
  type Pseud,
  type RegisterInput,
  activatePseud,
  fetchMe,
  register,
  signIn,
  signOut,
} from './api';

/** `unknown` until the server has actually answered. */
export type SessionStatus = 'unknown' | 'anonymous' | 'signed-in';

export class SessionStore {
  /** Whether anybody is signed in, as far as we have been told. */
  status = $state<SessionStatus>('unknown');

  /** The account, its pseuds and its capabilities; null when anonymous. */
  me = $state<MeResponse | null>(null);

  /** Why the last attempt to ask failed, when it was not a 401. */
  error = $state<unknown>(null);

  /** De-duplicates concurrent refreshes. Deliberately not reactive. */
  private inFlight: Promise<void> | null = null;

  get isSignedIn(): boolean {
    return this.status === 'signed-in';
  }

  get pseuds(): Pseud[] {
    return this.me?.pseuds ?? [];
  }

  get activePseud(): Pseud | null {
    const me = this.me;
    if (!me) return null;
    const id = me.active_pseud_id;
    if (!id) return me.pseuds[0] ?? null;
    return me.pseuds.find((pseud) => pseud.id === id) ?? null;
  }

  /**
   * Ask the server who we are.
   *
   * Writes state but deliberately never reads it: this is called from an
   * effect during boot, and a read here would make the effect depend on the
   * value it is about to write.
   */
  async refresh(): Promise<void> {
    if (this.inFlight) return this.inFlight;

    const attempt = this.load().finally(() => {
      this.inFlight = null;
    });
    this.inFlight = attempt;
    return attempt;
  }

  private async load(): Promise<void> {
    try {
      const me = await fetchMe();
      this.me = me;
      this.status = 'signed-in';
      this.error = null;
    } catch (failure) {
      if (failure instanceof ApiError && failure.isAuthRequired) {
        this.me = null;
        this.status = 'anonymous';
        this.error = null;
        return;
      }
      this.me = null;
      this.status = 'unknown';
      this.error = failure;
    }
  }

  /** Create an account and sign in. Throws so the form can show field errors. */
  async registerWith(input: RegisterInput): Promise<void> {
    await register(input);
    // The response already proves a session exists, so a refresh here cannot
    // fail into "anonymous" by accident — it is the same cookie jar.
    await this.refresh();
  }

  /** Sign in. Throws so the form can show field errors. */
  async signInWith(email: string, password: string): Promise<void> {
    await signIn(email, password);
    await this.refresh();
  }

  /**
   * End this session.
   *
   * Local state is cleared even if the request fails: a reader who asked to be
   * signed out must not be left looking at a signed-in page because the
   * network was down. The failure is recorded rather than thrown, because the
   * caller has nothing useful to do with it — the page simply says that the
   * session may still be live on the server.
   */
  async signOutNow(): Promise<void> {
    let failure: unknown = null;
    try {
      await signOut();
    } catch (error) {
      failure = error;
    }
    this.me = null;
    this.status = 'anonymous';
    this.error = failure;
  }

  /** Choose which pseud this session acts as, then re-read the session. */
  async usePseud(id: string): Promise<void> {
    await activatePseud(id);
    await this.refresh();
  }
}

/** The one store the application uses. */
export const session = new SessionStore();
