<script lang="ts">
  import { ApiError, completePasswordReset, requestPasswordReset } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import TextField from '../lib/components/TextField.svelte';

  /** `ask` collects the address; `finish` redeems the token it produced. */
  let step = $state<'ask' | 'finish'>('ask');

  let email = $state('');
  let token = $state('');
  let newPassword = $state('');

  /** The server's deliberately non-committal answer to step one. */
  let message = $state('');
  /** Present only outside production; see the note below. */
  let developmentToken = $state('');

  let done = $state(false);
  let error = $state<unknown>(null);
  let fieldErrors = $state<Record<string, string>>({});
  let submitting = $state(false);

  async function ask(event: SubmitEvent) {
    event.preventDefault();
    submitting = true;
    error = null;
    fieldErrors = {};
    try {
      const started = await requestPasswordReset(email.trim());
      message = started.message;
      developmentToken = started.development_token ?? '';
      // With no mail transport configured the token comes back in the response
      // as well as going to the log, so the flow is completable on a
      // development instance. Prefill it rather than making it a scavenger hunt.
      if (developmentToken) {
        token = developmentToken;
        step = 'finish';
      }
    } catch (failure) {
      error = failure;
      if (failure instanceof ApiError) fieldErrors = failure.fieldErrors;
    } finally {
      submitting = false;
    }
  }

  async function finish(event: SubmitEvent) {
    event.preventDefault();
    submitting = true;
    error = null;
    fieldErrors = {};
    try {
      await completePasswordReset(token.trim(), newPassword);
      newPassword = '';
      // Redeeming the token ends every session, including this one if the
      // reader was signed in. The shell has to be told, or it keeps claiming a
      // session the server has already revoked.
      await session.refresh();
      done = true;
    } catch (failure) {
      error = failure;
      if (failure instanceof ApiError) fieldErrors = failure.fieldErrors;
    } finally {
      submitting = false;
    }
  }
</script>

<section class="page">
  <h1>Reset your password</h1>

  {#if done}
    <div class="done" role="status">
      <h2>Password changed</h2>
      <p>
        Every session on the account has ended, including any you were not using. Sign in
        again with the new password.
      </p>
      <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>Sign in</a>
    </div>
  {:else if step === 'ask'}
    <p class="lede">
      Enter the address on the account. The answer is the same whether or not it exists.
    </p>

    {#if error}
      <ErrorSummary {error} />
    {/if}
    {#if message}
      <p class="note" role="status">{message}</p>
    {/if}

    <form onsubmit={ask} novalidate>
      <TextField
        id="reset-email"
        label="Email address"
        type="email"
        autocomplete="email"
        required
        value={email}
        error={fieldErrors.email}
        oninput={(event) => (email = event.currentTarget.value)}
      />

      <div class="actions">
        <Button type="submit" loading={submitting}>Send a reset link</Button>
        <Button variant="quiet" onclick={() => (step = 'finish')}>I already have a token</Button>
      </div>
    </form>
  {:else}
    <p class="lede">
      A reset link carries a single-use token. Redeeming it ends every session on the
      account.
    </p>

    {#if developmentToken}
      <p class="note" role="status">
        This instance has no mail transport configured, so the token was returned in the
        response instead of being emailed. It is shown here for that reason and nowhere
        else.
      </p>
    {/if}

    {#if error}
      <ErrorSummary {error} />
    {/if}

    <form onsubmit={finish} novalidate>
      <TextField
        id="reset-token"
        label="Reset token"
        required
        value={token}
        error={fieldErrors.token}
        oninput={(event) => (token = event.currentTarget.value)}
      />

      <TextField
        id="reset-password"
        label="New password"
        type="password"
        autocomplete="new-password"
        required
        value={newPassword}
        hint="At least 10 characters."
        error={fieldErrors.new_password}
        oninput={(event) => (newPassword = event.currentTarget.value)}
      />

      <div class="actions">
        <Button type="submit" loading={submitting}>Set a new password</Button>
        <Button variant="quiet" onclick={() => (step = 'ask')}>Ask for a new link</Button>
      </div>
    </form>
  {/if}
</section>

<style>
  .page {
    max-width: 34rem;
  }

  h1 {
    font-family: var(--font-heading);
    margin-bottom: var(--space-3);
  }

  .lede {
    color: var(--color-muted);
    margin-bottom: var(--space-5);
    max-width: 52ch;
  }

  .note {
    font-size: var(--text-sm);
    border-left: 3px solid var(--color-accent);
    background: var(--color-accent-soft);
    padding: var(--space-3);
    border-radius: var(--radius-sm);
    margin: 0 0 var(--space-4);
  }

  .done {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    background: var(--color-surface);
    max-width: 52ch;
  }

  .done h2 {
    margin-top: 0;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
</style>
