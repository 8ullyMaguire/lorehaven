<script lang="ts">
  /**
   * The writing desk: everything this pseud has written, and anything waiting
   * on it.
   *
   * The list is the *acting pseud's* work, because that is what the server
   * answers with: switching pseuds changes this page, which is the visible
   * consequence of ownership belonging to a pseud rather than an account.
   */
  import {
    ApiError,
    createWork,
    fetchInvitations,
    fetchWorks,
    respondToInvitation,
    type Invitation,
    type WorkSummary,
  } from '../lib/api';
  import { describeLifecycle, describeVisibility } from '../lib/labels';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import TextField from '../lib/components/TextField.svelte';

  let works = $state<WorkSummary[] | null>(null);
  let invitations = $state<Invitation[] | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(false);

  let newTitle = $state('');
  let creating = $state(false);
  let createError = $state<unknown>(null);

  let answering = $state<string | null>(null);
  let invitationError = $state<unknown>(null);

  $effect(() => {
    if (session.isSignedIn) void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [list, pending] = await Promise.all([fetchWorks(), fetchInvitations()]);
      works = list;
      invitations = pending;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function startWork(event: SubmitEvent) {
    event.preventDefault();
    creating = true;
    createError = null;
    try {
      const work = await createWork(newTitle.trim());
      newTitle = '';
      window.location.assign(`/write/${work.id}`);
    } catch (failure) {
      createError = failure;
    } finally {
      creating = false;
    }
  }

  async function answer(invitation: Invitation, accept: boolean) {
    answering = invitation.id;
    invitationError = null;
    try {
      await respondToInvitation(invitation.id, accept);
      await load();
    } catch (failure) {
      invitationError = failure;
    } finally {
      answering = null;
    }
  }
</script>

<h1>Write</h1>

{#if !session.isSignedIn}
  <EmptyState
    title="Sign in to write"
    description="Lorehaven keeps reading open, but publishing needs an account so that a work has an author."
  />
  <p>
    <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a>
    ·
    <a href="/register" onclick={(event) => handleLinkClick(event, '/register')}>Create an account</a>
  </p>
{:else}
  <p class="acting">
    Writing as <strong>@{session.activePseud?.handle ?? 'no pseud selected'}</strong>. Works belong
    to the pseud that created them; switching pseud changes this list.
  </p>

  {#if error}
    <ErrorSummary error={error} />
  {/if}

  <form class="new-work" onsubmit={startWork}>
    <TextField label="New work" bind:value={newTitle} placeholder="A title" required />
    <Button type="submit" disabled={creating || newTitle.trim() === ''}>Start a draft</Button>
  </form>
  {#if createError}
    <ErrorSummary error={createError} />
  {/if}

  {#if invitations && invitations.length > 0}
    <section aria-labelledby="invitations-heading">
      <h2 id="invitations-heading">Invitations</h2>
      {#if invitationError}
        <ErrorSummary error={invitationError} />
      {/if}
      <ul class="invitations">
        {#each invitations as invitation (invitation.id)}
          <li>
            <span>
              <strong>@{invitation.invited_by_handle}</strong> invites you to help with
              <em>{invitation.work_title}</em> as {invitation.role_label}.
            </span>
            {#if invitation.message}<span class="note">“{invitation.message}”</span>{/if}
            <div class="invitation-actions">
              <Button
                size="sm"
                disabled={answering === invitation.id}
                onclick={() => void answer(invitation, true)}>Accept</Button
              >
              <Button
                variant="quiet"
                size="sm"
                disabled={answering === invitation.id}
                onclick={() => void answer(invitation, false)}>Decline</Button
              >
            </div>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section aria-labelledby="works-heading">
    <h2 id="works-heading">Your works</h2>

    {#if loading && works === null}
      <Skeleton lines={3} />
    {:else if works && works.length === 0}
      <EmptyState
        title="Nothing here yet"
        description="Start a draft above, and it will appear here with its chapters and revision history."
      />
    {:else if works}
      <ul class="works">
        {#each works as work (work.id)}
          <li>
            <div class="work-main">
              <a href={`/write/${work.id}`} onclick={(event) => handleLinkClick(event, `/write/${work.id}`)}>
                {work.title.trim() === '' ? 'Untitled' : work.title}
              </a>
              <span class="meta">
                {describeLifecycle(work.lifecycle)} · {describeVisibility(work.visibility)} ·
                {work.chapter_count}
                {work.chapter_count === 1 ? 'chapter' : 'chapters'} · {work.word_count} words ·
                role: {work.role}
              </span>
              {#if work.lifecycle === 'published'}
                <a class="read-link" href={`/works/${work.id}`} onclick={(event) => handleLinkClick(event, `/works/${work.id}`)}>
                  Read it
                </a>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .acting {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .new-work {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    margin: var(--space-4) 0 var(--space-6);
  }

  ul {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .invitations li,
  .works li {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .work-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .work-main > a {
    font-family: var(--font-heading);
    font-size: var(--text-lg);
    text-decoration: none;
  }

  .meta,
  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .invitation-actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
