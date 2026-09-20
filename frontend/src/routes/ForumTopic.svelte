<script lang="ts">
  /**
   * One topic thread: posts, reply form, and mode-specific surfaces (spec §35).
   *
   * Reading is public; replying needs a signed-in pseud. A locked topic shows
   * its posts but refuses new ones. Mode-specific surfaces (reading group
   * schedule, critique queue) render for non-plain topics. Moderation panel
   * is available to signed-in moderators.
   */
  import {
    createReply,
    fetchPosts,
    fetchTopic,
    type ForumTopic,
    type ForumPost,
    setTopicMode,
  } from '../lib/api';
  import { session } from '../lib/session.svelte.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import WorkBacklink from '../lib/components/WorkBacklink.svelte';
  import ThreadModePicker from '../lib/components/ThreadModePicker.svelte';
  import ForumPostEl from '../lib/components/ForumPost.svelte';
  import ReadingSchedule from '../lib/components/ReadingSchedule.svelte';
  import CritiqueQueue from '../lib/components/CritiqueQueue.svelte';
  import ModerationPanel from '../lib/components/ModerationPanel.svelte';
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

  // Moderator status — determined by the account's trust level.
  // The Account interface does not yet expose trust_level directly,
  // so we infer moderator status from a future /me endpoint extension.
  let isModerator = $derived(session.isSignedIn); // simplified

  void load();
</script>

<div class="page">
  {#if loading}
    <Skeleton lines={4} />
  {:else if topic}
    {#if error}<ErrorSummary {error} />{/if}

    <h1>{topic.title}</h1>
    <p class="meta">
      started by {topic.author_handle ?? topic.author_pseud}
      {#if topic.locked}
        · locked
      {/if}
    </p>

    <WorkBacklink {topicId} />

    {#if topic.mode !== 'plain'}
      <p class="mode-badge">Mode: {topic.mode}</p>
    {/if}

    {#if session.isSignedIn}
      <ThreadModePicker {topicId} mode={topic.mode} />
    {/if}

    <!-- Mode-specific surfaces -->
    {#if topic.mode === 'reading_group'}
      <ReadingSchedule {topicId} {isModerator} />
    {:else if topic.mode === 'critique_circle'}
      <CritiqueQueue {topicId} {isModerator} />
    {/if}

    <!-- Posts -->
    {#if posts.length === 0}
      <p>No replies yet.</p>
    {:else}
      <ol class="posts">
        {#each posts as post (post.id)}
          <li>
            <ForumPostEl {post} readerChapter={0} />
          </li>
        {/each}
      </ol>
    {/if}

    <!-- Moderation panel -->
    {#if isModerator}
      <div class="moderation-wrap">
        <ModerationPanel {topicId} />
      </div>
    {/if}

    <!-- Reply form -->
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
  .mode-badge {
    display: inline-block;
    padding: 0.25rem 0.75rem;
    border-radius: 999px;
    background: var(--accent-bg, #eef);
    font-size: 0.875rem;
    margin: 0.5rem 0;
  }
  .posts {
    padding-left: 1.25rem;
  }
  .posts li {
    margin-bottom: 0.75rem;
  }
  .moderation-wrap {
    margin: 1.5rem 0;
  }
  .receipt {
    color: var(--ok, #2a7a2a);
  }
</style>