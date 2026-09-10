<script lang="ts">
  /**
   * The chapter editor (spec §8.3).
   *
   * The Tiptap schema here is the *client* half of the contract enforced on the
   * server: the same node and mark names, and nothing else. If the two ever
   * disagree, the server refuses the save and the editor says so — which is the
   * right direction for the disagreement to surface.
   *
   * Three things this component is careful about:
   *
   * 1. **The version is never invented.** Every save sends the version the
   *    server last returned, so a second tab cannot silently overwrite this
   *    one; a conflict is shown, with the local text kept.
   * 2. **A link target is checked before it is applied.** `javascript:` never
   *    reaches the document, because the server would refuse the whole save and
   *    the author would have no idea which keystroke caused it.
   * 3. **The recovery copy is offered, not applied.** Restoring a browser-local
   *    draft over the server's version is the writer's decision.
   */
  import { onDestroy, untrack } from 'svelte';
  import { Editor } from '@tiptap/core';
  import StarterKit from '@tiptap/starter-kit';
  import Link from '@tiptap/extension-link';

  import {
    ApiError,
    fetchRevisions,
    restoreRevision,
    updateChapter,
    type RevisionEntry,
  } from '../api';
  import { ChapterAutosave, type SaveState } from '../autosave';
  import Button from './Button.svelte';
  import TextField from './TextField.svelte';

  interface Props {
    chapterId: string;
    workId: string;
    /** The chapter version the server most recently gave us. */
    version: number;
    /** The stored editor document, if there is one. */
    document: unknown | null;
    title: string;
    /** Called after a successful save, with the new version and word count. */
    onsaved?: (version: number, wordCount: number) => void;
    /** Called when the title changes. */
    onrenamed?: (title: string) => void;
  }

  let { chapterId, workId, version, document, title, onsaved, onrenamed }: Props = $props();

  let container = $state<HTMLDivElement | null>(null);
  let editor = $state<Editor | null>(null);

  let saveState = $state<SaveState>('idle');
  let saveMessage = $state<string | undefined>(undefined);
  let wordCount = $state(0);
  /**
   * The title field starts from the chapter's title and then belongs to the
   * writer; it is not a mirror of the prop.
   */
  let titleDraft = $state(untrack(() => title));
  let note = $state('');

  let revisions = $state<RevisionEntry[] | null>(null);
  let showHistory = $state(false);
  let historyError = $state<unknown>(null);
  let restoring = $state(false);

  let recovered = $state<{ document: unknown; savedAt: number } | null>(null);

  /**
   * The links this editor will accept.
   *
   * The server's rule, restated on the client so that the refusal happens where
   * the author can see what caused it.
   */
  const ALLOWED_LINK = /^(https?:\/\/[^\s<>"'`]+|mailto:[^\s<>"'`]+|\/[^\s<>"'`]*)$/i;

  /*
   * One autosave per mounted editor, deliberately: the parent keys this
   * component by chapter, so switching chapters builds a new one. Reading the
   * props through `untrack` says so out loud — an autosave that silently
   * followed a *later* chapter would save into the wrong one.
   */
  const autosave = untrack(() => autosaveFor(version));

  function autosaveFor(startVersion: number) {
    return new ChapterAutosave(startVersion, {
    storageKey: `lorehaven:chapter:${chapterId}:draft`,
    storage: typeof window === 'undefined' ? null : window.localStorage,
    save: async (payload) => {
      const saved = await updateChapter(chapterId, {
        expected_version: payload.expected_version,
        document: payload.document,
        note: note.trim() || undefined,
      });
      onsaved?.(saved.version, saved.word_count);
      return { version: saved.version, word_count: saved.word_count };
    },
      onState: (state, detail) => {
        saveState = state;
        saveMessage = detail;
      },
    });
  }

  /** The document the editor starts from. */
  function startingDocument(): unknown {
    if (document) return document;
    return { type: 'doc', content: [{ type: 'paragraph' }] };
  }

  function countWords(text: string): number {
    return text.split(/\s+/).filter((word) => word.length > 0).length;
  }

  $effect(() => {
    if (!container) return;

    const instance = new Editor({
      element: container,
      extensions: [
        StarterKit.configure({
          // Only the nodes the server's schema knows about. Code blocks, inline
          // code and strikethrough are off because the schema does not have
          // them, and an author should not be offered what cannot be stored.
          codeBlock: false,
          code: false,
          strike: false,
          dropcursor: false,
          gapcursor: false,
          heading: { levels: [1, 2, 3, 4] },
        }),
        Link.configure({ openOnClick: false, autolink: false }),
      ],
      content: startingDocument() as never,
      onUpdate: ({ editor: instance }) => {
        wordCount = countWords(instance.getText());
        autosave.update(instance.getJSON());
      },
      onCreate: ({ editor: instance }) => {
        wordCount = countWords(instance.getText());
      },
    });

    editor = instance;
    recovered = autosave.recover();

    return () => {
      instance.destroy();
      editor = null;
    };
  });

  // The browser is the only reliable source for "we are offline".
  $effect(() => {
    if (typeof window === 'undefined') return;
    const onOffline = () => autosave.markOffline();
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      if (autosave.hasUnsavedWork) event.preventDefault();
    };
    window.addEventListener('offline', onOffline);
    window.addEventListener('beforeunload', onBeforeUnload);
    return () => {
      window.removeEventListener('offline', onOffline);
      window.removeEventListener('beforeunload', onBeforeUnload);
    };
  });

  onDestroy(() => {
    editor?.destroy();
  });

  function setLink() {
    const instance = editor;
    if (!instance) return;
    const previous = (instance.getAttributes('link').href as string | undefined) ?? '';
    const answer = window.prompt('Link address', previous);
    if (answer === null) return;
    const href = answer.trim();
    if (href === '') {
      instance.chain().focus().extendMarkRange('link').unsetLink().run();
      return;
    }
    if (!ALLOWED_LINK.test(href)) {
      saveMessage = 'A link must start with http://, https://, mailto: or /.';
      saveState = 'error';
      return;
    }
    instance.chain().focus().extendMarkRange('link').setLink({ href }).run();
  }

  async function loadHistory() {
    showHistory = !showHistory;
    if (!showHistory || revisions) return;
    historyError = null;
    try {
      revisions = await fetchRevisions(chapterId);
    } catch (failure) {
      historyError = failure;
    }
  }

  async function restore(revisionId: string) {
    restoring = true;
    historyError = null;
    try {
      const restored = await restoreRevision(chapterId, revisionId);
      const loaded = await fetchChapterDocument(restored.id);
      const instance = editor;
      if (instance && loaded) instance.commands.setContent(loaded as never);
      revisions = await fetchRevisions(chapterId);
      autosave.setVersion(restored.version);
      onsaved?.(restored.version, restored.word_count);
    } catch (failure) {
      historyError = failure;
    } finally {
      restoring = false;
    }
  }

  /**
   * Reload the chapter's text after a restore.
   *
   * A restore appends a revision rather than rewriting one, so the text to
   * show is the *new* current revision, not the one that was picked.
   */
  async function fetchChapterDocument(_chapterId: string): Promise<unknown | null> {
    const response = await fetch(`/api/v1/works/${workId}/chapters/${chapterId}`, {
      headers: { accept: 'application/json' },
    });
    if (!response.ok) return null;
    const body = (await response.json()) as { document?: unknown };
    return body.document ?? null;
  }

  function acceptRecovered() {
    const instance = editor;
    const draft = recovered;
    if (instance && draft) instance.commands.setContent(draft.document as never);
    autosave.discardRecovery();
    recovered = null;
    if (instance) autosave.update(instance.getJSON());
  }

  function discardRecovered() {
    autosave.discardRecovery();
    recovered = null;
  }

  async function renameChapter() {
    if (titleDraft === title) return;
    try {
      await updateChapter(chapterId, {
        expected_version: autosave.currentVersion,
        title: titleDraft,
      });
      onrenamed?.(titleDraft);
    } catch (failure) {
      saveState = 'error';
      saveMessage =
        failure instanceof ApiError ? failure.message : 'The chapter could not be renamed.';
    }
  }

  const statusLabel = $derived(
    saveState === 'saving'
      ? 'Saving…'
      : saveState === 'saved'
        ? 'Saved'
        : saveState === 'pending'
          ? 'Unsaved changes'
          : saveState === 'offline'
            ? 'Offline'
            : saveState === 'conflict'
              ? 'Conflict'
              : saveState === 'error'
                ? 'Save failed'
                : 'Up to date',
  );
</script>

<section class="editor" aria-label="Chapter editor">
  {#if recovered}
    <div class="banner" role="status">
      <p>
        A draft of this chapter was left in this browser and has not been sent to the server.
        Restoring it will replace what is loaded here.
      </p>
      <div class="banner-actions">
        <Button size="sm" onclick={acceptRecovered}>Restore local draft</Button>
        <Button variant="quiet" size="sm" onclick={discardRecovered}>Discard it</Button>
      </div>
    </div>
  {/if}

  <div class="chapter-meta">
    <TextField label="Chapter title" bind:value={titleDraft} onblur={renameChapter} />
    <TextField label="Revision note (optional)" bind:value={note} />
  </div>

  <div class="toolbar" role="toolbar" aria-label="Formatting">
    <Button
      variant="quiet"
      size="sm"
      onclick={() => editor?.chain().focus().toggleBold().run()}
      aria-pressed={editor?.isActive('bold') ?? false}>Bold</Button
    >
    <Button
      variant="quiet"
      size="sm"
      onclick={() => editor?.chain().focus().toggleItalic().run()}
      aria-pressed={editor?.isActive('italic') ?? false}>Italic</Button
    >
    <Button
      variant="quiet"
      size="sm"
      onclick={() => editor?.chain().focus().toggleHeading({ level: 2 }).run()}
      aria-pressed={editor?.isActive('heading', { level: 2 }) ?? false}>Heading</Button
    >
    <Button
      variant="quiet"
      size="sm"
      onclick={() => editor?.chain().focus().toggleBulletList().run()}
      aria-pressed={editor?.isActive('bulletList') ?? false}>Bullets</Button
    >
    <Button
      variant="quiet"
      size="sm"
      onclick={() => editor?.chain().focus().toggleOrderedList().run()}
      aria-pressed={editor?.isActive('orderedList') ?? false}>Numbers</Button
    >
    <Button
      variant="quiet"
      size="sm"
      onclick={() => editor?.chain().focus().toggleBlockquote().run()}
      aria-pressed={editor?.isActive('blockquote') ?? false}>Quote</Button
    >
    <Button
      variant="quiet"
      size="sm"
      onclick={() => editor?.chain().focus().setHorizontalRule().run()}>Scene break</Button
    >
    <Button variant="quiet" size="sm" onclick={setLink}>Link</Button>
  </div>

  <div class="surface" bind:this={container}></div>

  <div class="status">
    <span class="state state-{saveState}" data-testid="save-state">{statusLabel}</span>
    <span class="words">{wordCount} words in this draft</span>
    <Button variant="quiet" size="sm" onclick={() => void autosave.flush()} disabled={saveState === 'saving'}>
      Save now
    </Button>
    <Button variant="quiet" size="sm" onclick={loadHistory}>Revision history</Button>
  </div>

  {#if saveMessage}
    <p class="message" role="status">{saveMessage}</p>
  {/if}

  {#if showHistory}
    <div class="history">
      {#if historyError}
        <p class="message">{historyError instanceof ApiError ? historyError.message : 'The history could not be loaded.'}</p>
      {:else if revisions === null}
        <p class="message">Loading…</p>
      {:else if revisions.length === 0}
        <p class="message">Nothing saved yet.</p>
      {:else}
        <ul>
          {#each revisions as revision (revision.id)}
            <li>
              <span>
                #{revision.revision_number} · {revision.word_count} words · {revision.author_handle}
                {#if revision.current}<strong> · current</strong>{/if}
              </span>
              {#if revision.note}<span class="note">{revision.note}</span>{/if}
              {#if !revision.current}
                <Button variant="quiet" size="sm" disabled={restoring} onclick={() => void restore(revision.id)}>
                  Restore
                </Button>
              {/if}
            </li>
          {/each}
        </ul>
        <p class="message">
          Restoring brings the old text back as a <em>new</em> revision. Nothing already written is
          rewritten.
        </p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .banner {
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-accent);
    border-radius: var(--radius-md);
    padding: var(--space-3) var(--space-4);
  }

  .banner p {
    margin: 0 0 var(--space-2);
  }

  .banner-actions {
    display: flex;
    gap: var(--space-2);
  }

  .chapter-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .surface {
    background: var(--color-bg);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    min-height: 18rem;
    font-family: var(--font-reading);
    line-height: 1.7;
  }

  .surface :global(.tiptap) {
    outline: none;
  }

  .surface :global(p) {
    margin: 0 0 var(--space-3);
  }

  .surface :global(blockquote) {
    border-left: 3px solid var(--color-accent);
    margin: var(--space-3) 0;
    padding-left: var(--space-3);
    color: var(--color-muted);
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .state {
    font-weight: 600;
  }

  .state-conflict,
  .state-error {
    color: var(--color-danger, #a33);
  }

  .state-saved {
    color: var(--color-primary);
  }

  .message {
    font-size: var(--text-sm);
    color: var(--color-muted);
    margin: 0;
  }

  .history ul {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .history li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }

  .note {
    color: var(--color-muted);
  }
</style>
