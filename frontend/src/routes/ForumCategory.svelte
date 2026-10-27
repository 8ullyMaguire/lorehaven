<script lang="ts">
  /**
   * One forum category: its topics and a form to start one.
   *
   * The category id comes from `/community/forums/<id>`. **Both the list and the
   * form are behind a session**: `get_forums_topics` is `RequireSession`
   * (`community.rs`), and `/community` says so plainly to a visitor. This page
   * used to say the list was public and then render the API's `AUTH_REQUIRED`
   * refusal as a red "That did not work" panel above an empty list — two
   * contradictory statements on one screen, one of them a raw error code. A
   * signed-out visitor now gets the same plain sentence the hub gives, and the
   * error panel is kept for the failures that are actually failures.
   */
  import { ApiError, fetchTopics, type ForumTopic } from '../lib/api';
  import { session } from '../lib/session.svelte.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import { handleLinkClick } from '../lib/router';
  import NewTopicForm from '../lib/components/NewTopicForm.svelte';

  let { categoryId }: { categoryId: string } = $props();

  let topics = $state<ForumTopic[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);

  /*
   * `AUTH_REQUIRED` is not a fault: it is the door. `error instanceof ApiError`
   * rather than a message match, because the server's code is the stable fact.
   */
  let authRequired = $derived(error instanceof ApiError && error.isAuthRequired);

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
  {:else if authRequired}
    <h1>Topics</h1>
    <p>
      Sign in to see the topics in this category. The forums are for members, so reading and
      posting both need an account.
    </p>
    <p>
      <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a>
      or
      <a href="/register" onclick={(event) => handleLinkClick(event, '/register')}>register</a>. The
      <a href="/community" onclick={(event) => handleLinkClick(event, '/community')}>community hub</a>
      explains what is behind the door.
    </p>
  {:else}
    {#if error}<ErrorSummary {error} onretry={() => void load()} />{/if}

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
