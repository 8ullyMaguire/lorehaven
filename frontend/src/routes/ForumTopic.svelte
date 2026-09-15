<script lang="ts">
  /**
   * One topic thread: the posts so far and a reply form.
   *
   * Reading is public; replying needs a signed-in pseud. A locked topic shows
   * its posts but refuses new ones — the lock is the server's word, and the
   * form is hidden to match.
   */
  import { createReply, fetchPosts, fetchTopic, type ForumPost, type ForumTopic } from '../lib/api';
  import { session } from '../lib/session.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import { handleLinkClick } from '../lib/router';

  let { topicId }: { topicId: string } = $props();

  let topic = $state<ForumTopic | null>(null);
  let posts = $state<ForumPost[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let draft = $state('');
  let posting = $state(false);
  let posted = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      topic = await fetchTopic(topicId);
      posts = await fetchPosts(topicId);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function reply(event: SubmitEvent) {
    event.preventDefault();
    const body = draft.trim();
    if (!body || posting) return;
    posting = true;
    error = null;
    try {
      await createReply(topicId, body);
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
  {:else if topic}
    {#if error}<ErrorSummary {error} />{/if}

    <h1>{topic.title}</h1>
    <p class="meta">
      started by {topic.author_pseud}
      {#if topic.locked}
        · locked
      {/if}
    </p>

    {#if posts.length === 0}
      <p>No replies yet.</p>
    {:else}
      <ol class="posts">
        {#each posts as post (post.id)}
          <li>
            <span class="author">{post.author_pseud}</span>
            <span class="when">{new Date(post.created_at).toLocaleString()}</span>
            <p>{post.body}</p>
          </li>
        {/each}
      </ol>
    {/if}

    {#if topic.locked}
      <p>This topic is locked; new replies are closed.</p>
    {:else if session.isSignedIn}
      <form onsubmit={reply}>
        <h2>Reply</h2>
        <label for="reply-body">Your reply</label>
        <textarea id="reply-body" rows="4" bind:value={draft} required></textarea>
        <button type="submit" disabled={posting || !draft.trim()}>
          {posting ? 'Posting…' : 'Post reply'}
        </button>
        {#if posted}<p class="receipt">Reply posted.</p>{/if}
      </form>
    {:else}
      <p>
        <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a>
        to reply.
      </p>
    {/if}
  {/if}
</div>

<style>
  .meta {
    opacity: 0.7;
  }
  .posts {
    padding-left: 1.25rem;
  }
  .posts li {
    margin-bottom: 0.75rem;
  }
  .author {
    font-weight: 600;
    margin-right: 0.5rem;
  }
  .when {
    opacity: 0.7;
    font-size: 0.875rem;
  }
  .receipt {
    color: var(--ok, #2a7a2a);
  }
</style>
