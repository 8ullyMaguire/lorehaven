<script lang="ts">
  /**
   * One forum category: its topics and a form to start one.
   *
   * The category id comes from `/community/forums/<id>`; the topic list is
   * public, while starting a topic needs a signed-in pseud.
   */
  import { fetchTopics, type ForumTopic } from '../lib/api';
  import { session } from '../lib/session.svelte.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import { handleLinkClick } from '../lib/router';
  import NewTopicForm from '../lib/components/NewTopicForm.svelte';

  let { categoryId }: { categoryId: string } = $props();

  let topics = $state<ForumTopic[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);

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
            >
              {topic.title}
              {#if topic.mode && topic.mode !== 'plain'}
                <span class="mode-tag">{topic.mode}</span>
              {/if}
            </a>
            <span class="meta">started by {topic.author_handle ?? topic.author_pseud}</span>
          </li>
        {/each}
      </ul>
    {/if}

    {#if session.isSignedIn}
      <NewTopicForm {categoryId} onCreated={load} />
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
  .mode-tag {
    display: inline-block;
    padding: 0.1rem 0.4rem;
    margin-left: 0.5rem;
    border-radius: 999px;
    background: var(--accent-bg, #eef);
    font-size: 0.75rem;
    text-transform: capitalize;
  }
</style>
