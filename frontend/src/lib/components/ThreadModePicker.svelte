<script lang="ts">
  /**
   * Thread mode picker (spec §35.3).
   *
   * Each topic carries a mode that restructures one surface. The mode is data:
   * a plain topic is unaffected, and a reading-group topic gets a schedule of
   * sections. Only the topic author or a moderator can change the mode.
   */
  import { fetchTopic, setTopicMode } from '../lib/api';
  import { session } from '../lib/session.svelte';
  import ErrorSummary from './ErrorSummary.svelte';

  let { topicId, mode: currentMode }: { topicId: string; mode: string } = $props();

  const MODES = [
    { value: 'plain', label: 'Plain', desc: 'Standard discussion thread' },
    { value: 'reading_group', label: 'Reading Group', desc: 'Scheduled chapters with discussion' },
    { value: 'critique_circle', label: 'Critique Circle', desc: 'Queue-based peer review' },
    { value: 'wiki_pin', label: 'Wiki Pin', desc: 'Collaborative pinned note' },
    { value: 'prompt', label: 'Prompt', desc: 'Writing prompt thread' },
  ] as const;

  let selected = $state(currentMode);
  let saving = $state(false);
  let error = $state<unknown>(null);
  let saved = $state(false);

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (selected === currentMode || saving) return;
    saving = true;
    error = null;
    saved = false;
    try {
      await setTopicMode(topicId, selected);
      saved = true;
      currentMode = selected;
    } catch (failure) {
      error = failure;
    } finally {
      saving = false;
    }
  }
</script>

{#if error}<ErrorSummary {error} />{/if}
<form class="mode-picker" onsubmit={save}>
  <fieldset>
    <legend>Thread mode</legend>
    {#each MODES as m}
      <label class="mode-option">
        <input type="radio" name="mode" bind={selected} value={m.value} disabled={saving} />
        <span class="mode-name">{m.label}</span>
        <span class="mode-desc">{m.desc}</span>
      </label>
    {/each}
  </fieldset>
  <button type="submit" disabled={saving || selected === currentMode}>
    {saving ? 'Saving…' : 'Set mode'}
  </button>
  {#if saved}<span class="receipt">Saved.</span>{/if}
</form>

<style>
  .mode-picker {
    margin: 1rem 0;
  }
  fieldset {
    border: 1px solid var(--border, #ccc);
    border-radius: 0.5rem;
    padding: 0.75rem;
  }
  legend {
    font-weight: 600;
    padding: 0 0.5rem;
  }
  .mode-option {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    padding: 0.35rem 0;
    cursor: pointer;
  }
  .mode-name {
    font-weight: 500;
    min-width: 7rem;
  }
  .mode-desc {
    opacity: 0.7;
    font-size: 0.9rem;
  }
  .receipt {
    color: var(--ok, #2a7a2a);
    margin-left: 0.5rem;
  }
</style>
