<script lang="ts">
  /**
   * The sign-in note, in one place.
   *
   * Nine pages used to open with `ErrorSummary` reading "That did not work /
   * authentication required" to a visitor who had not asked to do anything. The
   * message is a correct report of a 401 and a useless thing to open a page with:
   * the reader cannot act on "sign in" if they did not know the page needed a
   * session, and `ErrorSummary`'s retry button re-issues a request that will fail
   * the same way.
   *
   * Two working patterns already existed in the tree before this component:
   *
   *   - **Guard before fetching** — `Library.svelte`, `History.svelte`,
   *     `Import.svelte`, `Exports.svelte` and `Jobs.svelte` all render
   *     `{#if !session.isSignedIn}` before touching the API, and those five pages
   *     are absent from the defect list. That is the right pattern and this is it,
   *     extracted so nine copies of the same sentence become one.
   *   - **Else read the status** — `AnalyticsDashboard.svelte` catches 401/403 and
   *     sets a flag, which is the fallback for a page that cannot avoid fetching on
   *     mount. It also catches the race where a page mounts before
   *     `session.refresh()` has answered, which is why the `pending` state is a
   *     third branch here rather than being folded into either.
   *
   * Nothing is fetched while `status` is `'unknown'`: that is the boot window, and
   * a fetch issued in it is the 401 this whole component exists to avoid. `App.svelte`
   * asks once at boot, so the window is one request long.
   */
  import type { Snippet } from 'svelte';
  import { handleLinkClick } from '../router';
  import { session } from '../session.svelte.ts';
  import Skeleton from './Skeleton.svelte';

  interface Props {
    /**
     * What the reader would get if they signed in — one clause, in this page's own
     * words. "see the works you have imported", "compare four works and calibrate
     * your taste". It is the whole point of the note: the reader has to be able to
     * decide whether signing in is worth it.
     */
    purpose: string;
    /** Render a skeleton while the session is still being asked about. */
    skeletonLines?: number;
    /** The page, shown once the reader is known to be signed in. */
    children: Snippet;
  }

  let { purpose, skeletonLines = 4, children }: Props = $props();
</script>

{#if session.status === 'unknown'}
  <Skeleton lines={skeletonLines} label="Checking your session" />
{:else if !session.isSignedIn}
  <p class="note" data-testid="signin-note">
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a>
    to {purpose}.
  </p>
{:else}
  {@render children()}
{/if}

<style>
  .note {
    color: var(--text-muted);
    max-width: var(--measure);
  }
</style>
