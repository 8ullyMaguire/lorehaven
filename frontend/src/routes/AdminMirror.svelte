<script lang="ts">
  /**
   * Local mirror & IPFS management (spec §32.7.6).
   *
   * For operators: register a local mirror for a media reference (saving the
   * bytes to disk) and pin the content to IPFS. Both surfaces share the
   * reference lookup: first find a reference by hash, then attach mirrors.
   */

  import {
    fetchLocalMirrors,
    addLocalMirror,
    deactivateLocalMirror,
    fetchIpfsPins,
    addIpfsPin,
    reverseMediaSearch,
    type ReverseSearchView,
    type LocalMirrorView,
    type IpfsPinView,
      } from '../lib/api';
      import Button from '../lib/components/Button.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import TextField from '../lib/components/TextField.svelte';

  // Search state
  let hashInput = $state('');
  let searchResult = $state<ReverseSearchView | null>(null);
  let searchLoading = $state(false);
  let searchError = $state<unknown>(null);

  // Active reference for mirroring
  let activeRefId = $state<string | null>(null);
  let mirrors = $state<LocalMirrorView[] | null>(null);
  let pins = $state<IpfsPinView[] | null>(null);
  let mirrorsLoading = $state(false);
  let pinsLoading = $state(false);

  // Mirror form
  let mirrorPath = $state('');
  let mirrorUrl = $state('');
  let mirrorBusy = $state(false);
  let mirrorError = $state<unknown>(null);

  // IPFS form
  let pinCid = $state('');
  let pinService = $state('');
  let pinSize = $state<number>(0);
  let pinBusy = $state(false);
  let pinError = $state<unknown>(null);

  async function onSearch(e: SubmitEvent) {
    e.preventDefault();
    if (!hashInput.trim()) return;
    searchLoading = true;
    searchError = null;
    searchResult = null;
    activeRefId = null;
    mirrors = null;
    pins = null;
    try {
      searchResult = await reverseMediaSearch({ hash: hashInput.trim() });
    } catch (failure) {
      searchError = failure;
    } finally {
      searchLoading = false;
    }
  }

  async function selectReference(referenceId: string) {
    activeRefId = referenceId;
    mirrorsLoading = true;
    pinsLoading = true;
    mirrors = null;
    pins = null;
    try {
      mirrors = await fetchLocalMirrors(referenceId);
    } catch (e) {
      mirrorError = e;
    } finally {
      mirrorsLoading = false;
    }
    try {
      pins = await fetchIpfsPins(referenceId);
    } catch (e) {
      pinError = e;
    } finally {
      pinsLoading = false;
    }
  }

  async function onMirrorSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (!activeRefId || !mirrorPath || !mirrorUrl) return;
    mirrorBusy = true;
    mirrorError = null;
    try {
      await addLocalMirror({
        media_reference_id: activeRefId,
        storage_path: mirrorPath,
        original_url: mirrorUrl,
      });
      // Refresh list
      mirrors = await fetchLocalMirrors(activeRefId);
      mirrorPath = '';
      mirrorUrl = '';
    } catch (failure) {
      mirrorError = failure;
    } finally {
      mirrorBusy = false;
    }
  }

  async function onPinSubmit(e: SubmitEvent) {
    e.preventDefault();
    if (!activeRefId || !pinCid || !pinService) return;
    pinBusy = true;
    pinError = null;
    try {
      await addIpfsPin({
        media_reference_id: activeRefId,
        cid: pinCid,
        pin_service: pinService,
        file_size_bytes: pinSize,
      });
      pins = await fetchIpfsPins(activeRefId);
      pinCid = '';
      pinService = '';
      pinSize = 0;
    } catch (failure) {
      pinError = failure;
    } finally {
      pinBusy = false;
    }
  }

  async function onDeactivate(mirrorId: string) {
    await deactivateLocalMirror(mirrorId);
    if (activeRefId) mirrors = await fetchLocalMirrors(activeRefId);
  }
</script>

<svelte:head><title>Mirror & Pin Management · Lorehaven</title></svelte:head>

<section class="page">
  <header>
    <p class="eyebrow">ADMIN</p>
    <h1>Mirror & Pin Management</h1>
    <p class="lede">
      Locate a media reference by hash, then register local mirrors or IPFS pins.
    </p>
  </header>

  <form class="search-form" onsubmit={onSearch}>
    <TextField label="Perceptual hash" bind:value={hashInput} placeholder="e.g. 8f3a2b1c..." />
    <Button type="submit" loading={searchLoading}>Search</Button>
  </form>

  {#if searchLoading}
    <Skeleton lines={3} />
  {:else if searchError}
    <ErrorSummary error={searchError} />
  {:else if searchResult}
    {#if searchResult.references.length === 0}
      <EmptyState title="No matching media" description="No media reference matched that hash." />
    {:else}
      <section class="results">
        <h2>References found</h2>
        <ul class="ref-list">
          {#each searchResult.references as ref (ref.id)}
            <li class="ref-row" class:active={activeRefId === ref.id}>
              <div class="ref-info">
                <code>{ref.perceptual_hash ?? ref.content_hash.slice(0, 16)}</code>
                <span class="badge">{ref.media_kind}</span>
                {#if ref.curator_verified}
                  <span class="badge badge-ok">Curator verified</span>
                {/if}
              </div>
              {#if activeRefId === ref.id}
                <Button size="sm" variant="quiet" disabled>Selected</Button>
              {:else}
                <Button size="sm" variant="primary" onclick={() => selectReference(ref.id)}>
                  Manage
                </Button>
              {/if}
            </li>
          {/each}
        </ul>
      </section>

      {#if activeRefId}
        <section class="mirror-section">
          <h2>Local Mirrors</h2>
          {#if mirrorsLoading}
            <Skeleton lines={2} />
          {:else if mirrorError}
            <ErrorSummary error={mirrorError} />
          {:else if mirrors && mirrors.length > 0}
            <table class="mirror-table">
              <thead>
                <tr>
                  <th>Storage Path</th>
                  <th>Status</th>
                  <th>Last Verified</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {#each mirrors as mirror (mirror.id)}
                  <tr>
                    <td><code>{mirror.storage_path}</code></td>
                    <td><span class="status {mirror.status}">{mirror.status}</span></td>
                    <td class="muted">{mirror.last_verified_at ?? 'Never'}</td>
                    <td>
                      {#if mirror.status === 'active'}
                        <Button size="sm" variant="danger" onclick={() => onDeactivate(mirror.id)}>
                          Deactivate
                        </Button>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <EmptyState title="No mirrors" description="No local mirrors registered for this reference." />
          {/if}

          <form class="add-form" onsubmit={onMirrorSubmit}>
            <h3>Add Mirror</h3>
            <TextField label="Storage path" bind:value={mirrorPath} placeholder="/data/media/ref-abc.jpg" />
            <TextField label="Original URL" bind:value={mirrorUrl} placeholder="https://..." />
            <Button type="submit" loading={mirrorBusy}>Register Mirror</Button>
          </form>
        </section>

        <section class="pin-section">
          <h2>IPFS Pins</h2>
          {#if pinsLoading}
            <Skeleton lines={2} />
          {:else if pinError}
            <ErrorSummary error={pinError} />
          {:else if pins && pins.length > 0}
            <ul class="pin-list">
              {#each pins as pin (pin.id)}
                <li class="pin-row">
                  <div class="pin-main">
                    <code>{pin.cid}</code>
                    <span class="badge">{pin.pin_service}</span>
                    <span class="status {pin.status}">{pin.status}</span>
                    {#if pin.file_size_bytes > 0}
                      <span class="muted">{Math.round(pin.file_size_bytes / 1024)} KB</span>
                    {/if}
                  </div>
                  <span class="muted">{pin.pinned_at}</span>
                </li>
              {/each}
            </ul>
          {:else}
            <EmptyState title="No IPFS pins" description="No pins registered for this reference." />
          {/if}

          <form class="add-form" onsubmit={onPinSubmit}>
            <h3>Add IPFS Pin</h3>
            <TextField label="CID" bind:value={pinCid} placeholder="bafy..." />
            <TextField label="Pin service" bind:value={pinService} />
            <TextField label="File size (bytes)" bind:value={pinSize} />
            <Button type="submit" loading={pinBusy}>Pin</Button>
          </form>
        </section>
      {/if}
    {/if}
  {/if}
</section>

<style>
  .page {
    max-width: 900px;
    margin: 0 auto;
    padding: var(--space-6);
  }
  .eyebrow {
    color: var(--color-muted);
    font-size: 0.75rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    margin-bottom: var(--space-1);
  }
  h1 {
    margin-bottom: var(--space-2);
  }
  .lede {
    color: var(--color-muted);
    margin-bottom: var(--space-6);
  }
  .search-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }
  h2 {
    margin-top: var(--space-6);
    margin-bottom: var(--space-3);
    font-size: 1.1rem;
  }
  .ref-list {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .ref-row {
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .ref-row.active {
    border-color: var(--color-accent);
    background: var(--color-accent-bg);
  }
  .ref-info {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  code {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    background: var(--color-surface-alt);
    padding: 2px 6px;
    border-radius: 4px;
  }
  .badge {
    font-size: 0.75rem;
    padding: 2px 8px;
    border-radius: 999px;
    font-weight: 500;
    background: var(--color-surface-alt);
    color: var(--color-muted);
  }
  .badge-ok {
    background: var(--color-success-bg);
    color: var(--color-success);
  }
  .status {
    font-size: 0.75rem;
    padding: 2px 8px;
    border-radius: 999px;
    font-weight: 500;
    background: var(--color-surface-alt);
    color: var(--color-muted);
  }
  .status.active {
    background: var(--color-success-bg);
    color: var(--color-success);
  }
  .status.inactive {
    background: var(--color-error-bg);
    color: var(--color-error);
  }
  .muted {
    color: var(--color-muted);
    font-size: 0.85rem;
  }
  .mirror-table {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: var(--space-4);
  }
  th,
  td {
    text-align: left;
    padding: var(--space-2);
    border-bottom: 1px solid var(--color-border);
    font-size: 0.85rem;
  }
  .pin-list {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .pin-row {
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .pin-main {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .add-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    margin-bottom: var(--space-6);
  }
  h3 {
    font-size: 1rem;
    font-weight: 600;
  }
</style>
