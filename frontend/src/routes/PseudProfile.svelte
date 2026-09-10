<script lang="ts">
  import { ApiError, fetchPublicPseud, type PublicPseud } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { formatTimestamp } from '../lib/time';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  interface Props {
    /** The handle from the path, without the leading @. */
    handle: string;
  }

  let { handle }: Props = $props();

  let pseud = $state<PublicPseud | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let missing = $state(false);

  async function load() {
    loading = true;
    error = null;
    missing = false;
    try {
      pseud = await fetchPublicPseud(handle);
    } catch (failure) {
      // A hidden pseud is a 404, not a 403: the site does not confirm that a
      // profile exists but is withheld. The page says the same thing.
      if (failure instanceof ApiError && failure.code === 'NOT_FOUND') missing = true;
      else error = failure;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    // Re-read when the handle changes, which happens on client-side navigation
    // between two profiles.
    void handle;
    void load();
  });
</script>

{#if loading}
  <Skeleton lines={4} label={`Loading @${handle}`} />
{:else if missing}
  <EmptyState
    title={`No pseud called @${handle}`}
    description="A profile that exists but is hidden is reported the same way, so this page never confirms who is here."
  >
    {#snippet action()}
      <a href="/" onclick={(event) => handleLinkClick(event, '/')}>Back to the front page</a>
    {/snippet}
  </EmptyState>
{:else if error}
  <ErrorSummary {error} onretry={load} />
{:else if pseud}
  <article class="profile">
    <header>
      <h1>{pseud.display_name}</h1>
      <p class="handle">@{pseud.handle}</p>
    </header>

    {#if pseud.bio}
      <p class="bio">{pseud.bio}</p>
    {:else}
      <p class="bio empty">This pseud has not written a biography.</p>
    {/if}

    <dl class="facts">
      <dt>On Lorehaven since</dt>
      <dd>{formatTimestamp(pseud.created_at)}</dd>
    </dl>

    <p class="note">
      Works, bookmarks and followers appear here once the milestones that hold them are
      built. Nothing on this page links this pseud to any other.
    </p>
  </article>
{/if}

<style>
  .profile {
    max-width: 52rem;
  }

  h1 {
    font-family: var(--font-heading);
    margin: 0;
  }

  .handle {
    color: var(--color-muted);
    margin: var(--space-1) 0 var(--space-5);
  }

  .bio {
    white-space: pre-wrap;
    max-width: 60ch;
    font-size: var(--text-lg);
  }

  .bio.empty {
    color: var(--color-muted);
    font-size: var(--text-base);
  }

  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-1) var(--space-4);
    font-size: var(--text-sm);
    margin: var(--space-5) 0;
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--color-border);
  }

  .facts dt {
    color: var(--color-muted);
  }

  .facts dd {
    margin: 0;
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
    max-width: 60ch;
  }
</style>
