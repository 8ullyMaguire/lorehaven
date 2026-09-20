<script lang="ts">
  /**
   * New topic creation form (spec §35.3).
   *
   * Lets a writer start a discussion with a chosen thread mode (Plain by
   * default). Mode selection is only available when creating the topic —
   * once set, the mode is fixed (it can be changed later by a moderator).
   */
  import { createTopicWithMode } from '../api';

  let { categoryId, onCreated }: { categoryId: string; onCreated?: () => void } = $props();

  let title = $state('');
  let mode = $state('plain');
  let posting = $state(false);
  let error = $state<string | null>(null);

  const modes = [
    { value: 'plain', label: 'Plain', desc: 'Free-form discussion' },
    { value: 'reading_group', label: 'Reading Group', desc: 'Scheduled chapter releases' },
    { value: 'critique_circle', label: 'Critique Circle', desc: 'Peer review queue' },
    { value: 'wiki', label: 'Wiki', desc: 'Collaborative knowledge base' },
    { value: 'prompt', label: 'Prompt', desc: 'Creative writing challenge' },
  ];

  async function start(event: SubmitEvent) {
    event.preventDefault();
    const t = title.trim();
    if (!t || posting) return;
    posting = true;
    error = null;
    try {
      await createTopicWithMode(categoryId, t, mode);
      title = '';
      mode = 'plain';
      onCreated?.();
    } catch (e) {
      error = e instanceof Error ? e.message : 'Could not start topic.';
    } finally {
      posting = false;
    }
  }
</script>

<form onsubmit={start} class="new-topic-form">
  <h2>Start a topic</h2>
  <label for="ntf-title">Title</label>
  <input id="ntf-title" bind:value={title} required maxlength={200} />
  <fieldset class="mode-picker">
    <legend>Thread mode</legend>
    {#each modes as m}
      <label>
        <input type="radio" bind:group={mode} value={m.value} />
        <span class="mode-name">{m.label}</span>
        <span class="mode-desc">{m.desc}</span>
      </label>
    {/each}
  </fieldset>
  {#if error}<p class="error">{error}</p>{/if}
  <button type="submit" disabled={posting || !title.trim()}>
    {posting ? 'Posting…' : 'Start topic'}
  </button>
</form>

<style>
  .new-topic-form {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .mode-picker {
    border: 1px solid var(--border, #ccc);
    border-radius: 4px;
    padding: 0.5rem;
  }
  .mode-picker label {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
  }
  .mode-name {
    font-weight: 600;
  }
  .mode-desc {
    opacity: 0.7;
    font-size: 0.875rem;
  }
  .error {
    color: var(--danger, #b00020);
  }
</style>