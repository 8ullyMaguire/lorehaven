<script lang="ts">
  /**
   * Reply form with spoiler zone awareness (spec §35.4).
   *
   * When a topic has spoiler_scope_chapter set, warns the poster about
   * potential spoilers. Also supports content warnings attachment.
   */
  import { createReply } from '../api';

  let {
    topicId,
    spoilerScopeChapter,
    onCreated,
  }: {
    topicId: string;
    spoilerScopeChapter?: number | null;
    onCreated?: () => void;
  } = $props();

  let body = $state('');
  let posting = $state(false);
  let error = $state<string | null>(null);
  let addWarning = $state(false);
  let warningType = $state('spoilers');
  let warningSeverity = $state(1);

  const warningTypes = [
    'violence',
    'sexual_content',
    'self_harm',
    'spoilers',
    'custom',
  ];

  async function reply(event: SubmitEvent) {
    event.preventDefault();
    const b = body.trim();
    if (!b || posting) return;
    posting = true;
    error = null;
    try {
      await createReply(topicId, b);
      body = '';
      addWarning = false;
      onCreated?.();
    } catch (e) {
      error = e instanceof Error ? e.message : 'Could not post reply.';
    } finally {
      posting = false;
    }
  }
</script>

<form onsubmit={reply} class="reply-form">
  <h2>Reply</h2>

  {#if spoilerScopeChapter}
    <p class="spoiler-warning">
      ⚠ This topic is scoped to chapter {spoilerScopeChapter}. Posts that
      reference content beyond that chapter should use a spoiler tag.
    </p>
  {/if}

  <label for="reply-body">Your reply</label>
  <textarea id="reply-body" rows="4" bind:value={body} required></textarea>

  <label class="warning-toggle">
    <input type="checkbox" bind:checked={addWarning} />
    Add content warning
  </label>

  {#if addWarning}
    <div class="warning-fields">
      <label for="warning-type">Type</label>
      <select id="warning-type" bind:value={warningType}>
        {#each warningTypes as wt}
          <option value={wt}>{wt.replace('_', ' ')}</option>
        {/each}
      </select>
      <label for="warning-severity">Severity</label>
      <input
        id="warning-severity"
        type="range"
        min="1"
        max="3"
        bind:value={warningSeverity}
      />
    </div>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}
  <button type="submit" disabled={posting || !body.trim()}>
    {posting ? 'Posting…' : 'Post reply'}
  </button>
</form>

<style>
  .reply-form {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 1rem;
  }
  .spoiler-warning {
    background: var(--accent-bg, #eef);
    padding: 0.5rem;
    border-radius: 0.25rem;
    font-size: 0.875rem;
  }
  .warning-toggle {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  .warning-fields {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    padding: 0.5rem;
    border: 1px solid var(--border, #ddd);
    border-radius: 0.25rem;
  }
  .error {
    color: var(--danger, #b00020);
  }
</style>