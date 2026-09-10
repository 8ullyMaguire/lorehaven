<script lang="ts">
  import { ApiError, type RegisterInput } from '../lib/api';
  import { AGE_BANDS } from '../lib/labels';
  import { handleLinkClick, navigate } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Select from '../lib/components/Select.svelte';
  import TextField from '../lib/components/TextField.svelte';

  let email = $state('');
  let password = $state('');
  let handle = $state('');
  let displayName = $state('');
  let ageBand = $state('unknown');

  let error = $state<unknown>(null);
  let submitting = $state(false);

  /** Field errors, taken from the server's `field_errors` envelope. */
  let fieldErrors = $state<Record<string, string>>({});

  function fieldError(name: string): string | undefined {
    return fieldErrors[name];
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    submitting = true;
    error = null;
    fieldErrors = {};

    const input: RegisterInput = {
      email: email.trim(),
      password,
      handle: handle.trim(),
      age_band: ageBand,
    };
    // Omitted rather than sent empty: the server defaults it to the handle.
    if (displayName.trim()) input.display_name = displayName.trim();

    try {
      await session.registerWith(input);
      password = '';
      navigate('/account');
    } catch (failure) {
      error = failure;
      if (failure instanceof ApiError) fieldErrors = failure.fieldErrors;
    } finally {
      submitting = false;
    }
  }
</script>

<section class="page">
  <h1>Create an account</h1>
  <p class="lede">
    An account is a place to keep a library and to write. Reading does not require one:
    everything the instance allows a visitor is available without signing in.
  </p>

  {#if error}
    <ErrorSummary {error} />
  {/if}

  <form onsubmit={submit} novalidate>
    <TextField
      id="register-email"
      label="Email address"
      type="email"
      autocomplete="email"
      required
      value={email}
      error={fieldError('email')}
      oninput={(event) => (email = event.currentTarget.value)}
    />

    <TextField
      id="register-password"
      label="Password"
      type="password"
      autocomplete="new-password"
      required
      value={password}
      hint="At least 10 characters. Length is the strength we check; there is no common-password list yet."
      error={fieldError('password')}
      oninput={(event) => (password = event.currentTarget.value)}
    />

    <TextField
      id="register-handle"
      label="Handle"
      required
      value={handle}
      hint="The name your first pseud is known by, without the @. Lower case letters, numbers, underscores and hyphens."
      error={fieldError('handle')}
      oninput={(event) => (handle = event.currentTarget.value)}
    />

    <TextField
      id="register-display-name"
      label="Display name"
      value={displayName}
      hint="Optional; defaults to the handle. Your pseuds are the names other people see, not your email address."
      error={fieldError('display_name')}
      oninput={(event) => (displayName = event.currentTarget.value)}
    />

    <Select
      id="register-age-band"
      label="Are you 18 or older?"
      options={AGE_BANDS.map((band) => ({ value: band.value, label: band.label }))}
      value={ageBand}
      hint="We do not collect birth dates, only a band. A statement is not a verification, and the account is treated accordingly."
      error={fieldError('age_band')}
      onchange={(event) => (ageBand = event.currentTarget.value)}
    />

    <div class="actions">
      <Button type="submit" loading={submitting}>Create account</Button>
      <a href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>
        I already have an account
      </a>
    </div>
  </form>
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

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    flex-wrap: wrap;
    margin-top: var(--space-2);
  }
</style>
