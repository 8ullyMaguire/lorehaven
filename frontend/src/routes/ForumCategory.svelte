<script lang="ts">
  /**
   * One forum category: its topics and a form to start one.
   *
   * The category id comes from `/community/forums/<id>`; the topic list is
   * public, while starting a topic needs a signed-in pseud. The trust gate
   * that certain categories enforce is the server's decision — the UI just
   * relays the answer.
   */
  import { createTopic, fetchTopics, type ForumTopic } from '../lib/api';
  import { session } from '../lib/session.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import { handleLinkClick } from '../lib/router';

  let { categoryId }: { categoryId: string } = $props();

  let topics = $state<ForumTopic[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let draft = $state('');
  let posting = $state(false);
  let posted = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      topics = await fetchTopics(categoryId);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function start(event: SubmitEvent) {
    event.preventDefault();
    const title = draft.trim();
    if (!title || posting) return;
    posting = true;
    error = null;
    try {
      await createTopic(categoryId, title);
      draft = '';
      posted = true;
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      posting = false;
    }
  }

  void load();
</script>

<div class="page">
  {#if loading}
    <Skeleton lines={4} />
  {:else}
    {#if error}<ErrorSummary {error} />{/if}

    <h1>Topics</h1>

    {#if topics.length === 0}
      <p>Nothing here yet. The first topic starts the conversation.</p>
    {:else}
      <ul class="topic-list">
        {#each topics as topic (topic.id)}
          <li>
            <a
              href={`/community/topics/${encodeURIComponent(topic.id)}`}
              onclick={(event) =>
                handleLinkClick(event, `/community/topics/${encodeURIComponent(topic.id)}`)}
            >{topic.title}</a>
            <span class="meta">started by {topic.author_pseud}</span>
          </li>
        {/each}
      </ul>
    {/if}

    {#if session.isSignedIn}
      <form onsubmit={start}>
        <h2>Start a topic</h2>
        <label for="topic-title">Title</label>
        <input id="topic-title" bind:value={draft} required />
        <button type="submit" disabled={posting || !draft.trim()}>
          {posting ? 'Posting…' : 'Start topic'}
        </button>
        {#if posted}<p class="receipt">Topic posted.</p>{/if}
      </form>
    {:else}
      <p>
        <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a>
        to start a topic.
      </p>
    {/if}
  {/if}
</div>

<style>
  .topic-list {
    list-style: none;
    padding: 0;
  }
  .topic-list li {
    margin-bottom: 0.5rem;
  }
  .meta {
    opacity: 0.7;
    margin-left: 0.5rem;
    font-size: 0.875rem;
  }
  .receipt {
    color: var(--ok, #2a7a2a);
  }
</style>
