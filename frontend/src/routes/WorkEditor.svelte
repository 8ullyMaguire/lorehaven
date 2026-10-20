<script lang="ts">
  /**
   * One work's editor: its metadata, its units, its contributors, and the
   * controls that change its publication state.
   *
   * Two things the interface does not do, deliberately:
   *
   * * It does not decide who may do what. `role` and `publication_blockers`
   *   come from the server, and the controls are rendered from them; a disabled
   *   button here is a courtesy, not a permission check (spec §3.6).
   * * It does not hide an empty draft behind a public-looking preview. A draft
   *   says it is a draft.
   */
  import {
    ApiError,
    addUnit,
    fetchUnit,
    fetchWork,
    inviteContributor,
    isAuthorWork,
    publishWork,
    removeContributor,
    reorderUnits,
    setDiscussionMode,
    updateContributor,
    updateWork,
    withdrawWork,
    type AuthorWork,
    type UnitContent,
    type Invitation,
  } from '../lib/api';
  import {
    CONTRIBUTOR_ROLES,
    RATINGS,
    describeCompletion,
    describeLifecycle,
    describeRating,
    describeRole,
    describeVisibility,
  } from '../lib/labels';
  import { deleteWorkPricing, fetchWorkPricing, setWorkPricing } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import PreReadPanel from '../lib/components/PreReadPanel.svelte';
  import Select from '../lib/components/Select.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import TextField from '../lib/components/TextField.svelte';
  import Textarea from '../lib/components/Textarea.svelte';

  interface Props {
    workId: string;
  }

  let { workId }: Props = $props();

  let work = $state<AuthorWork | null>(null);
  let notMine = $state(false);
  let error = $state<unknown>(null);
  let loading = $state(true);

  // Metadata form.
  let title = $state('');
  let summary = $state('');
  let language = $state('en');
  let rating = $state('general');
  let visibility = $state('public');
  let completion = $state('in_progress');
  let showRatings = $state(true);
  let savingMeta = $state(false);
  let metaError = $state<unknown>(null);
  let metaMessage = $state<string | null>(null);
  // Discussion mode form.
  let discussionMode = $state('thread_only');
  let discussionMessage = $state<string | null>(null);

  // Units.
  let openUnit = $state<string | null>(null);
  let unitContent = $state<UnitContent | null>(null);
  let unitLoading = $state(false);
  let newUnitTitle = $state('');
  let addingUnit = $state(false);
  let unitError = $state<unknown>(null);

  // Publication.
  let publishing = $state(false);
  let publishError = $state<unknown>(null);

  // Pricing (money decisions belong to the owner — spec §20.9).
  let pricingModel = $state('free');
  let pricingPrice = $state('');
  let pricingCurrency = $state('EUR');
  let savingPricing = $state(false);
  let pricingError = $state<unknown>(null);
  let pricingMessage = $state<string | null>(null);

  // Contributors.
  let inviteHandle = $state('');
  let inviteRole = $state('coauthor');
  let inviting = $state(false);
  let inviteError = $state<unknown>(null);
  let lastInvite = $state<Invitation | null>(null);

  /*
   * The editor is loaded on demand.
   *
   * It carries the rich-text engine, which is most of the bundle; a reader
   * never needs it, and an author does not need it until they open a unit.
   * Splitting it out is what keeps the reading pages small.
   */
  type EditorComponent = typeof import('../lib/components/UnitEditor.svelte').default;
  let UnitEditor = $state<EditorComponent | null>(null);
  let editorLoadFailed = $state(false);

  $effect(() => {
    if (UnitEditor || editorLoadFailed) return;
    void import('../lib/components/UnitEditor.svelte')
      .then((module) => {
        UnitEditor = module.default;
      })
      .catch(() => {
        editorLoadFailed = true;
      });
  });

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    notMine = false;
    try {
      const response = await fetchWork(workId);
      if (!isAuthorWork(response)) {
        // A published work read by someone who does not contribute to it.
        notMine = true;
        work = null;
        return;
      }
      apply(response);
      void loadPricing();
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  function apply(next: AuthorWork) {
    work = next;
    title = next.title;
    summary = next.summary;
    language = next.language;
    rating = next.rating;
    visibility = next.visibility;
    completion = next.completion;
    showRatings = next.show_public_ratings;
    discussionMode = next.discussion_mode;
  }

  const canEdit = $derived(work !== null && work.role !== 'beta_reader');
  const canPublish = $derived(work !== null && (work.role === 'owner' || work.role === 'coauthor'));

  async function saveMetadata(event: SubmitEvent) {
    event.preventDefault();
    const current = work;
    if (!current) return;
    savingMeta = true;
    metaError = null;
    metaMessage = null;
    try {
      const updated = await updateWork(current.id, {
        expected_version: current.version,
        title,
        summary,
        language,
        rating,
        visibility,
        completion,
        show_public_ratings: showRatings,
      });
      apply(updated);
      metaMessage = 'Saved.';
    } catch (failure) {
      metaError = failure;
      if (failure instanceof ApiError && failure.code === 'REVISION_CONFLICT') {
        metaMessage =
          'This work changed somewhere else. Reload to see the current version before saving again.';
      }
    } finally {
      savingMeta = false;
    }
  }

  /** Read the work's current pricing into the form, if any. */
  async function loadPricing() {
    const current = work;
    if (!current) return;
    try {
      const response = await fetchWorkPricing(current.id);
      const purchase = response.pricing?.find((p) => p.model === 'purchase');
      if (purchase) {
        pricingModel = 'purchase';
        pricingPrice = String(purchase.price_minor);
        pricingCurrency = purchase.currency;
      } else {
        pricingModel = 'free';
      }
    } catch {
      // No pricing row yet — the form's defaults already say "free".
      pricingModel = 'free';
    }
  }

  async function savePricing(event: SubmitEvent) {
    event.preventDefault();
    const current = work;
    if (!current) return;
    savingPricing = true;
    pricingError = null;
    pricingMessage = null;
    try {
      if (pricingModel === 'free') {
        await deleteWorkPricing(current.id);
        pricingMessage = 'The work reads free.';
      } else {
        const price = Number.parseInt(pricingPrice, 10);
        if (!Number.isFinite(price) || price <= 0) {
          pricingError = new Error('A purchase price must be a positive number of minor units.');
          return;
        }
        await setWorkPricing(current.id, {
          model: 'purchase',
          price_minor: price,
          currency: pricingCurrency.trim() || 'EUR',
          public_at_offset: null,
        });
        pricingMessage = 'Pricing saved.';
      }
    } catch (failure) {
      pricingError = failure;
    } finally {
      savingPricing = false;
    }
  }

  async function createUnit(event: SubmitEvent) {
    event.preventDefault();
    const current = work;
    if (!current) return;
    addingUnit = true;
    unitError = null;
    try {
      const unit = await addUnit(current.id, newUnitTitle.trim());
      newUnitTitle = '';
      await load();
      await open(unit.id);
    } catch (failure) {
      unitError = failure;
    } finally {
      addingUnit = false;
    }
  }

  async function open(unitId: string) {
    const current = work;
    if (!current) return;
    openUnit = unitId;
    unitContent = null;
    unitLoading = true;
    unitError = null;
    try {
      unitContent = await fetchUnit(current.id, unitId);
    } catch (failure) {
      unitError = failure;
    } finally {
      unitLoading = false;
    }
  }

  async function move(index: number, delta: number) {
    const current = work;
    if (!current) return;
    const target = index + delta;
    if (target < 0 || target >= current.units.length) return;
    const order = current.units.map((unit) => unit.id);
    const [moved] = order.splice(index, 1);
    order.splice(target, 0, moved);
    unitError = null;
    try {
      await reorderUnits(current.id, order);
      await load();
    } catch (failure) {
      unitError = failure;
    }
  }

  async function saveDiscussionMode(mode: string) {
    const current = work;
    if (!current) return;
    discussionMessage = null;
    try {
      await setDiscussionMode(current.id, mode);
      discussionMessage = 'Discussion mode updated.';
    } catch (failure) {
      discussionMessage = 'Could not update discussion mode.';
    }
  }

  async function publish() {
    const current = work;
    if (!current) return;
    publishing = true;
    publishError = null;
    try {
      // One key per attempt, reused on retry, so a double click cannot publish
      // and notify twice (spec §8 acceptance).
      const key = `ui-publish-${current.id}-${current.version}`;
      apply(await publishWork(current.id, current.version, key));
    } catch (failure) {
      publishError = failure;
    } finally {
      publishing = false;
    }
  }

  async function withdraw() {
    const current = work;
    if (!current) return;
    publishing = true;
    publishError = null;
    try {
      const key = `ui-withdraw-${current.id}-${current.version}`;
      apply(await withdrawWork(current.id, current.version, key));
    } catch (failure) {
      publishError = failure;
    } finally {
      publishing = false;
    }
  }

  async function invite(event: SubmitEvent) {
    event.preventDefault();
    const current = work;
    if (!current) return;
    inviting = true;
    inviteError = null;
    lastInvite = null;
    try {
      lastInvite = await inviteContributor(current.id, {
        handle: inviteHandle.trim(),
        role: inviteRole,
      });
      inviteHandle = '';
      await load();
    } catch (failure) {
      inviteError = failure;
    } finally {
      inviting = false;
    }
  }

  async function changeRole(pseudId: string, role: string) {
    const current = work;
    if (!current) return;
    inviteError = null;
    try {
      await updateContributor(current.id, pseudId, { role });
      await load();
    } catch (failure) {
      inviteError = failure;
    }
  }

  async function remove(pseudId: string) {
    const current = work;
    if (!current) return;
    inviteError = null;
    try {
      await removeContributor(current.id, pseudId);
      await load();
    } catch (failure) {
      inviteError = failure;
    }
  }
</script>

{#if loading}
  <Skeleton lines={4} />
{:else if notMine}
  <h1>Not your work</h1>
  <p>
    This work exists, but the pseud you are writing as does not contribute to it. You can read it
    as anyone else would.
  </p>
  <Button onclick={() => window.location.assign(`/works/${workId}`)}>Read it</Button>
{:else if error}
  <h1>Something went wrong</h1>
  <ErrorSummary error={error} />
{:else if work}
  <nav class="crumbs">
    <a href="/write" onclick={(event) => handleLinkClick(event, '/write')}>Write</a>
  </nav>

  <h1>{work.title.trim() === '' ? 'Untitled' : work.title}</h1>
  <p class="badges">
    <span>{describeLifecycle(work.lifecycle)}</span>
    <span>{describeVisibility(work.visibility)}</span>
    <span>{describeCompletion(work.completion)}</span>
    <span>{describeRating(work.rating)}</span>
    <span>your role: {describeRole(work.role)}</span>
  </p>

  <!--
    §32.6: a pre-read assessment belongs to the author, on the editor. There is deliberately
    no equivalent block in WorkPage.svelte, and the panel itself renders nothing on a 404,
    so a reader who reaches this route without owning the work gets no report and no
    message about one.
  -->
  {#if PreReadPanel}
    <PreReadPanel workId={work.id} />
  {/if}

  {#if work.lifecycle === 'published'}
    {@const publishedHref = `/works/${work.id}`}
    <p>
      <a href={publishedHref} onclick={(event) => handleLinkClick(event, publishedHref)}>
        Read the published version
      </a>
    </p>
  {/if}

  <section aria-labelledby="metadata-heading">
    <h2 id="metadata-heading">Details</h2>
    <form class="metadata" onsubmit={saveMetadata}>
      <TextField label="Title" bind:value={title} disabled={!canEdit} />
      <Textarea label="Summary" bind:value={summary} disabled={!canEdit} />
      <TextField label="Language" bind:value={language} disabled={!canEdit} />
      <Select
        label="Rating"
        value={rating}
        options={RATINGS.map((value) => ({ value, label: describeRating(value) }))}
        onchange={(event) => (rating = event.currentTarget.value)}
        disabled={!canEdit}
      />
      <Select
        label="Who can read it"
        value={visibility}
        options={['public', 'unlisted', 'restricted'].map((value) => ({
          value,
          label: describeVisibility(value),
        }))}
        onchange={(event) => (visibility = event.currentTarget.value)}
        disabled={!canEdit}
      />
      <Select
        label="Completion"
        value={completion}
        options={['in_progress', 'complete', 'hiatus', 'abandoned'].map((value) => ({
          value,
          label: describeCompletion(value),
        }))}
        onchange={(event) => (completion = event.currentTarget.value)}
        disabled={!canEdit}
      />
      <label class="checkbox">
        <input type="checkbox" bind:checked={showRatings} disabled={!canEdit} />
        Show the public rating summary on the work page
      </label>
      <div class="actions">
        <Button type="submit" disabled={savingMeta || !canEdit}>Save details</Button>
        {#if metaMessage}<span class="note">{metaMessage}</span>{/if}
      </div>
    </form>
    {#if metaError}<ErrorSummary error={metaError} />{/if}
  </section>

  <section aria-labelledby="discussion-heading">
    <h2 id="discussion-heading">Discussion</h2>
    <p class="note">How readers talk about this work. Reactions and a linked forum thread, inline comments, or both.</p>
    <Select
      label="Discussion mode"
      value={discussionMode}
      options={[
        { value: 'thread_only', label: 'Reactions and linked discussion' },
        { value: 'comments_only', label: 'Inline comments' },
        { value: 'both', label: 'Reactions, comments, and linked discussion' },
      ]}
      onchange={(event) => void saveDiscussionMode(event.currentTarget.value)}
      disabled={!canEdit}
    />
    {#if discussionMessage}<span class="note">{discussionMessage}</span>{/if}
  </section>

  <section aria-labelledby="pricing-heading">
    <h2 id="pricing-heading">Pricing</h2>
    <p class="note">Set a purchase price, or leave it off and the work reads free.</p>
    <form class="pricing" onsubmit={savePricing}>
      <Select
        label="Model"
        value={pricingModel}
        options={['free', 'purchase'].map((value) => ({ value, label: value }))}
        onchange={(event) => (pricingModel = event.currentTarget.value)}
        disabled={!canEdit}
      />
      {#if pricingModel === 'purchase'}
        <TextField label="Price (minor units, 500 = 5.00)" bind:value={pricingPrice} disabled={!canEdit} />
        <TextField label="Currency" bind:value={pricingCurrency} disabled={!canEdit} />
      {/if}
      <div class="actions">
        <Button type="submit" disabled={savingPricing || !canEdit}>Save pricing</Button>
        {#if pricingMessage}<span class="note">{pricingMessage}</span>{/if}
      </div>
    </form>
    {#if pricingError}<ErrorSummary error={pricingError} />{/if}
  </section>

  <section aria-labelledby="publish-heading">
    <h2 id="publish-heading">Publication</h2>
    {#if work.publication_blockers.length > 0}
      <ul class="blockers">
        {#each work.publication_blockers as blocker (blocker)}
          <li>{blocker}</li>
        {/each}
      </ul>
    {/if}
    <div class="actions">
      <Button
        disabled={publishing || !canPublish || work.publication_blockers.length > 0}
        onclick={() => void publish()}
      >
        {work.lifecycle === 'published' ? 'Republish' : 'Publish'}
      </Button>
      {#if work.lifecycle === 'published'}
        <Button variant="quiet" disabled={publishing || !canPublish} onclick={() => void withdraw()}>
          Withdraw
        </Button>
      {/if}
    </div>
    {#if !canPublish}
      <p class="note">Only an owner or co-author may change publication state.</p>
    {/if}
    {#if publishError}<ErrorSummary error={publishError} />{/if}
  </section>

  <section aria-labelledby="units-heading">
    <h2 id="units-heading">Units</h2>
    {#if unitError}<ErrorSummary error={unitError} />{/if}

    <form class="new-unit" onsubmit={createUnit}>
      <TextField label="New unit" bind:value={newUnitTitle} placeholder="Unit title" />
      <Button type="submit" disabled={addingUnit || !canEdit}>Add unit</Button>
    </form>

    {#if work.units.length === 0}
      <p class="note">No units yet. A work needs at least one with text before it can be published.</p>
    {:else}
      <ol class="units">
        {#each work.units as unit, index (unit.id)}
          <li class:open={openUnit === unit.id}>
            <div class="unit-row">
              <span class="unit-title">{unit.title.trim() === '' ? 'Untitled unit' : unit.title}</span>
              <span class="note">
                {unit.word_count} words · {unit.revision_count}
                {unit.revision_count === 1 ? 'revision' : 'revisions'}
              </span>
              <div class="unit-actions">
                <Button variant="quiet" size="sm" disabled={!canEdit} onclick={() => void move(index, -1)}>Up</Button>
                <Button variant="quiet" size="sm" disabled={!canEdit} onclick={() => void move(index, 1)}>Down</Button>
                <Button variant="quiet" size="sm" onclick={() => void open(unit.id)}>
                  {openUnit === unit.id ? 'Reload' : 'Edit'}
                </Button>
              </div>
            </div>

            {#if openUnit === unit.id}
              {#if unitLoading}
                <Skeleton lines={3} />
              {:else if unitContent}
                <!--
                  Keyed by unit: an editor holds one unit's text and one
                  autosave, so switching units must build a new one rather
                  than re-point the old.
                -->
                {#key unitContent.unit.id}
                  {#if UnitEditor}
                    <UnitEditor
                      unitId={unit.id}
                      workId={work.id}
                      version={unitContent.unit.version}
                      document={unitContent.document}
                      title={unitContent.unit.title}
                      onsaved={() => void load()}
                    />
                  {:else if editorLoadFailed}
                    <p class="note">
                      The editor could not be loaded. Your text is safe; reload the page to try
                      again.
                    </p>
                  {:else}
                    <p class="note">Loading the editor…</p>
                  {/if}
                {/key}
              {/if}
            {/if}
          </li>
        {/each}
      </ol>
    {/if}
  </section>

  <section aria-labelledby="contributors-heading">
    <h2 id="contributors-heading">Contributors</h2>
    {#if inviteError}<ErrorSummary error={inviteError} />{/if}
    {#if lastInvite}
      <p class="note">
        Invited <strong>@{lastInvite.invited_handle}</strong> as {lastInvite.role_label}. It is
        waiting for them to accept; nothing is granted until they do.
      </p>
    {/if}

    <ul class="contributors">
      {#each work.contributors as contributor (contributor.pseud_id)}
        <li>
          <span>
            <strong>@{contributor.handle}</strong> · {contributor.display_name} ·
            {describeRole(contributor.role)}
            {#if !contributor.public_attribution}<span class="note"> · credited privately</span>{/if}
          </span>
          {#if canPublish && contributor.role !== 'owner'}
            <Select
              label="Role"
              value={contributor.role}
              options={CONTRIBUTOR_ROLES.map((role) => ({ value: role.value, label: role.label }))}
              onchange={(event) => void changeRole(contributor.pseud_id, event.currentTarget.value)}
            />
            <Button variant="quiet" size="sm" onclick={() => void remove(contributor.pseud_id)}>
              Remove
            </Button>
          {/if}
        </li>
      {/each}
    </ul>

    {#if canPublish}
      <form class="invite" onsubmit={invite}>
        <TextField label="Invite a pseud by handle" bind:value={inviteHandle} placeholder="handle" />
        <Select
          label="Offered role"
          value={inviteRole}
          options={CONTRIBUTOR_ROLES.map((role) => ({ value: role.value, label: role.label }))}
          onchange={(event) => (inviteRole = event.currentTarget.value)}
        />
        <Button type="submit" disabled={inviting || inviteHandle.trim() === ''}>Send invitation</Button>
      </form>
      <p class="note">
        An invitation names pseuds, never accounts. Ownership cannot be handed over by invitation.
        There is no email transport yet, so the invited pseud sees it on their own Write page.
      </p>
    {/if}
  </section>
{/if}

<style>
  .crumbs {
    font-size: var(--text-sm);
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  section {
    margin-top: var(--space-6);
    padding-top: var(--space-4);
    border-top: var(--border-width) solid var(--color-border);
  }

  .metadata,
  .new-unit,
  .invite {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
  }

  .checkbox {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .blockers {
    color: var(--color-danger, #a33);
    font-size: var(--text-sm);
  }

  .units {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    counter-reset: unit;
  }

  .units > li {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    padding: var(--space-3);
  }

  .units > li.open {
    border-color: var(--color-accent);
  }

  .unit-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .unit-title {
    font-family: var(--font-heading);
    flex: 1;
  }

  .unit-actions {
    display: flex;
    gap: var(--space-2);
  }

  .contributors {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .contributors li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }
</style>
