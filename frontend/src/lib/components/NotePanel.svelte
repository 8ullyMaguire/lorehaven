<script lang="ts">
  /**
   * Private notes for a work (spec §9.2: "spoiler reveal, private notes").
   *
   * A note is never rendered inside the reading text — it belongs to the reader,
   * not to the work, and putting it in the prose would eventually put it in
   * front of someone else. So it lives in a panel beside the text, and the
   * server answers only for the acting pseud.
   */
  import { deleteNote, fetchNotes, saveNote, type NoteView } from '../api';
  import ErrorSummary from './ErrorSummary.svelte';

  interface Props {
    /** The subject a note belongs to: the work, not the chapter. */
    subjectType: 'work' | 'library_item';
    subjectId: string;
    signedIn: boolean;
  }

  let { subjectType, subjectId, signedIn }: Props = $props();

  let notes = $state<NoteView[]>([]);
  let draft = $state('');
  let loading = $state(false);
  let error = $state<unknown>(null);

  $effect(() => {
    if (!signedIn) return;
    void load();
  });

  async function load() {
    try {
      notes = await fetchNotes(subjectType, subjectId);
    } catch (failure) {
      error = failure;
    }
  }

  async function add() {
    if (draft.trim() === '') return;
    loading = true;
    error = null;
    try {
      await saveNote({ subject_type: subjectType, subject_id: subjectId, body: draft });
      draft = '';
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function remove(id: string) {
    try {
      await deleteNote(id);
      notes = notes.filter((note) => note.id !== id);
    } catch (failure) {
      error = failure;
    }
  }
</script>

<aside class="notes" aria-label="Your private notes">
  <h2>Your notes</h2>

  {#if !signedIn}
    <p class="note">Sign in to keep private notes. Nobody else can read them.</p>
  {:else}
    {#if notes.length === 0}
      <p class="note">No notes yet. They are private to you.</p>
    {:else}
      <ul>
        {#each notes as note (note.id)}
          <li>
            <p class="body">{note.body}</p>
            <button type="button" class="quiet" onclick={() => remove(note.id)}>
              Delete note
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <label for="note-draft" class="label">Add a note</label>
    <textarea id="note-draft" bind:value={draft} rows="3"></textarea>
    {#if error}
      <ErrorSummary error={error} />
    {/if}
    <button type="button" onclick={add} disabled={loading || draft.trim() === ''}>
      Add note
    </button>
  {/if}
</aside>

<style>
  .notes {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-3) var(--space-4);
    background: var(--color-surface);
    font-size: var(--text-sm);
  }

  .notes h2 {
    font-size: var(--text-base);
    margin: 0 0 var(--space-2);
  }

  .note {
    color: var(--color-muted);
  }

  .notes ul {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .notes li {
    border-left: 2px solid var(--color-accent);
    padding-left: var(--space-2);
  }

  .body {
    margin: 0 0 var(--space-1);
    white-space: pre-wrap;
  }

  .label {
    display: block;
    margin: var(--space-2) 0 var(--space-1);
  }

  textarea {
    width: 100%;
    font: inherit;
    color: var(--color-text);
    background: var(--color-bg);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    padding: var(--space-2);
  }

  .quiet {
    background: none;
    border: var(--border-width) solid var(--color-border);
    color: var(--color-muted);
    font-size: var(--text-sm);
  }
</style>
