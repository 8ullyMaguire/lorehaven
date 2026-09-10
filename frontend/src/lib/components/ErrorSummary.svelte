<script lang="ts">
  import { ApiError } from '../api';
  import Button from './Button.svelte';

  interface Props {
    error: unknown;
    onretry?: () => void;
  }

  let { error, onretry }: Props = $props();

  let api = $derived(error instanceof ApiError ? error : null);
  let code = $derived(api?.code ?? 'UNKNOWN');
  let message = $derived(api?.message ?? 'Something went wrong.');
  let requestId = $derived(api?.requestId ?? null);
  let fields = $derived(Object.entries(api?.fieldErrors ?? {}));
</script>

<!--
  Error summary (Milestone 1). Announced immediately, and it names the stable
  code and the request id so a reader can quote them to support without
  screenshots of a stack trace.
-->
<div class="summary" role="alert" aria-labelledby="error-summary-title">
  <h3 id="error-summary-title">That did not work</h3>
  <p class="message">{message}</p>

  {#if fields.length > 0}
    <ul class="fields">
      {#each fields as [field, detail] (field)}
        <li><strong>{field}</strong>: {detail}</li>
      {/each}
    </ul>
  {/if}

  <p class="meta">
    <span>Code <code>{code}</code></span>
    {#if requestId}<span>Request <code>{requestId}</code></span>{/if}
  </p>

  {#if onretry}
    <Button variant="secondary" size="sm" onclick={onretry}>Try again</Button>
  {/if}
</div>

<style>
  .summary {
    border: var(--border-width) solid var(--color-danger);
    border-left-width: 3px;
    border-radius: var(--radius-md);
    background: var(--color-surface);
    padding: var(--space-4);
    margin-bottom: var(--space-4);
  }

  h3 {
    margin: 0 0 var(--space-2);
    color: var(--color-danger);
    font-size: var(--text-lg);
  }

  .message {
    margin: 0 0 var(--space-3);
  }

  .fields {
    margin: 0 0 var(--space-3);
    padding-left: var(--space-5);
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    font-size: var(--text-sm);
    color: var(--color-muted);
    margin: 0 0 var(--space-3);
  }
</style>
