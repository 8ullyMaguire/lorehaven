<script lang="ts">
  import EmptyState from '../lib/components/EmptyState.svelte';

  interface Props {
    path: string;
  }

  let { path }: Props = $props();
</script>

<article class="not-found">
  <h1>No such page</h1>
  <p class="path">Lorehaven has nothing at <code>{path}</code>.</p>
  <EmptyState
    title="Nothing here"
    description="The address may be mistyped, or the work may have been withdrawn. A withdrawn work is indistinguishable from one that never existed, which is deliberate: the site does not confirm the existence of things you cannot read."
  />
  <p><a href="/">Return to the entrance</a></p>
</article>

<style>
  .not-found {
    padding-top: var(--space-6);
    max-width: 52ch;
  }

  .path {
    color: var(--color-muted);
  }
</style>
