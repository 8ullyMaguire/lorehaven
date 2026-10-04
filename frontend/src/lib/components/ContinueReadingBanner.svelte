<script module lang="ts">
  /**
   * Exported from the module context so tests can type fixtures against the real shape. An
   * `export interface` in the instance `<script>` is not importable in Svelte 5.
   */
  import type { ContinueReading } from '../api';
  export type { ContinueReading };

  /**
   * The props, exported so `ContinueReadingBanner.test.ts` can type its fixture helper.
   *
   * `ComponentProps<ContinueReadingBanner>` is not a substitute: it resolves to `undefined`
   * for a component whose props are declared in the instance script, so a helper typed with
   * it accepts nothing and every override becomes `not assignable to parameter of type
   * 'undefined'`.
   */
  export type ContinueReadingBannerProps = {
    /**
     * Injected rather than imported so the test can drive every branch — including the two
     * that matter most and cannot be reached by a real request: a server that is down, and
     * a request that was aborted because the reader navigated away.
     */
    load?: typeof fetchContinueReading;
    /** When false, the component renders nothing at all and makes no request. */
    signedIn?: boolean;
  };

  import { fetchContinueReading } from '../api';
  export { fetchContinueReading };
</script>

<script lang="ts">
  /**
   * "Continue Reading" — item 1 of the 100-idea audit, and the highest-impact retention
   * item on the list.
   *
   * ## What renders, and what does not
   *
   * One row, or nothing. Not a list: the feature is "know instantly where you left off",
   * which one row answers, and a three-item banner is a reading history nobody asked for.
   * `reading_progress` already existed and already had `position_permille` — this reads it.
   *
   * ## The three invisible states, which are most of the work
   *
   * A banner is a component that is usually absent, and each way of being absent is a
   * different bug:
   *
   *  - **Signed out** → nothing, and NO REQUEST. Not "request and hide the 401": an
   *    anonymous visitor on the homepage should not produce a failed request per page view.
   *  - **Nothing to continue** (`null`, i.e. the server's 404) → nothing. A reader who
   *    finished everything is not being scolded about it.
   *  - **The request failed** (500, timeout, offline) → nothing, silently. This is the
   *    decision most worth stating: a reader whose server is down should see a working
   *    homepage with no banner, NOT an error box about a retention feature they did not
   *    ask for. The banner is an enhancement; it must never be able to break the page.
   *
   * ## Why there is no skeleton
   *
   * A placeholder that occupies the banner's height on every page load, then vanishes,
   * moves the whole homepage down by its own height and back up again. On a page whose
   * first screen is a hero, that is a visible jump on every navigation. Better: arrive
   * already correct, or not at all.
   */

  let { load = fetchContinueReading, signedIn = true }: ContinueReadingBannerProps = $props();

  let entry = $state<ContinueReading | null>(null);
  let resolved = $state(false);

  /**
   * `$effect`, not `onMount` — and the reason is a bug this component shipped with.
   *
   * `signedIn` is false for TWO different reasons and they need opposite handling:
   *
   *   session 'unknown'    the server has not been asked yet. Asking is CORRECT. Returning
   *                        early is a bug, and it is the bug this had: the component
   *                        mounted in the same tick as the homepage, `session.status` was
   *                        still 'unknown', so `onMount` saw `isSignedIn === false` and
   *                        returned without ever fetching.
   *   session 'anonymous'  the server answered: there is nobody to ask about.
   *
   * `onMount` reads the value ONCE, at a moment when it is not yet known. `$effect`
   * re-runs when `signedIn` changes, so the request happens when the answer arrives — which
   * is the only moment it is safe to make.
   *
   * The unit tests could not see this, and that is the part worth keeping: every one of
   * them set `session.status` BEFORE rendering, so `isSignedIn` was already true at mount
   * and the race never existed. A test that arranges the world into a state the page is
   * never actually in cannot find a bug that only exists in the real order of events. The
   * browser's resource log named it in one line:
   *
   *   requests for continue-reading: []            <- never asked
   *   api requests this load: /api/v1/meta, /api/v1/auth/me
   *
   * `settled` guards the single fetch: `signedIn` flipping false again (a sign-out on
   * another tab) must not start a second request.
   */
  // NOT `$state`, and that is load-bearing. A `$state` read inside `$effect` is a
  // dependency, and this effect WRITES it, so it invalidates itself; Svelte stops the loop
  // and the early `return` fires with `settled` still false -- the banner then never
  // fetched at all, which is how seven of the ten tests here failed on the fix that was
  // supposed to make them pass. The guard is bookkeeping, not reactive input, so it is a
  // plain `let`.
  let settled = false;

  $effect(() => {
    // `signedIn` is the only reactive input read here, so this re-runs exactly when the
    // session resolves.
    const known = signedIn;
    if (!known) {
      resolved = true;
      return;
    }
    if (settled) return;
    settled = true;
    const controller = new AbortController();
    load(controller.signal)
      .then((got) => {
        if (controller.signal.aborted) return;
        entry = got;
      })
      .catch(() => {
        // Swallowed on purpose, and asserted as swallowed in the test suite. See the
        // component comment: an enhancement must not be able to break the page.
      })
      .finally(() => {
        if (!controller.signal.aborted) resolved = true;
      });
    return () => controller.abort();
  });
</script>

<!--
  The absence assertion needs its subject counted. This component has been the site of the
  exact false-green described in the project notes: `queryByTestId` returning `null` also
  passes when the component is not mounted at all, or when the element is inside a block
  that never rendered. So the test asserts the banner EXISTS first, in the populated case,
  and only then asserts it is ABSENT in the empty ones. `hidden` rather than an `{#if}`,
  so the node is in the DOM in every state and `inDocument` means what it says.
-->
<div
  data-testid="continue-reading-banner"
  hidden={!(entry && resolved)}
  aria-live="polite"
>
  {#if entry}
    <p class="eyebrow">Continue reading</p>
    <!--
      `/works/{id}`, NOT `/work/{id}`. The singular form is the shape of the API path
      (`/api/v1/works/...`), not the page path, and linking to it renders "No such page".
      The component test asserted `href === '/work/work-1'` because that is what the
      component produced -- a test that confirms the implementation instead of the contract.
      The E2E journey caught it, on the one assertion that actually navigates.
    -->
    <a class="title" data-testid="continue-reading-title" href={`/works/${entry.workId}`}
      >{entry.title}</a
    >
    {#if entry.chapterTitle}
      <p class="chapter" data-testid="continue-reading-chapter">{entry.chapterTitle}</p>
    {/if}
    <div
      class="bar"
      role="progressbar"
      aria-valuenow={entry.percent}
      aria-valuemin="0"
      aria-valuemax="100"
      aria-label={`Progress through ${entry.title}`}
    >
      <div class="fill" style={`width: ${entry.percent}%`}></div>
    </div>
    <p class="percent" data-testid="continue-reading-percent">{entry.percent}%</p>
  {/if}
</div>

<style>
  /*
   * Deliberately NOT visually hidden when empty. `hidden` removes it from layout, so the
   * homepage does not reserve the banner's height for a reader who has nothing to resume.
   * The one thing this must never do is push the hero down and then pull it back.
   */
  div[hidden] {
    display: none;
  }
  .eyebrow {
    margin: 0 0 0.15rem;
    font-size: 0.75rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    opacity: 0.7;
  }
  .title {
    font-weight: 600;
  }
  .chapter {
    margin: 0.15rem 0 0.4rem;
    font-size: 0.9rem;
    opacity: 0.8;
  }
  .bar {
    height: 4px;
    border-radius: 2px;
    background: currentColor;
    opacity: 0.2;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: currentColor;
    opacity: 1;
  }
  .percent {
    margin: 0.3rem 0 0;
    font-size: 0.8rem;
    opacity: 0.7;
  }
</style>
