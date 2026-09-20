<script lang="ts">
  /**
   * Content warning preferences (spec §35.4).
   *
   * Lets a reader set their default action (blur or show) for each
   * warning type. Applies site-wide across all forum posts.
   */
  import { getWarningPrefs, setWarningPref, type WarningPref } from '../lib/api';

  let prefs = $state<WarningPref[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);

  const WARNING_TYPES = [
    { value: 'violence', label: 'Graphic Violence' },
    { value: 'death', label: 'Major Character Death' },
    { value: 'nonconsensual', label: 'Non-consensual' },
    { value: 'underage', label: 'Underage' },
    { value: 'self_harm', label: 'Self-harm' },
    { value: 'eating_disorder', label: 'Eating Disorders' },
    { value: 'spoiler', label: 'Spoilers' },
    { value: 'flashing', label: 'Flashing Lights' },
    { value: 'loud_audio', label: 'Loud Audio' },
    { value: 'substance_abuse', label: 'Substance Abuse' },
  ];

  async function load() {
    loading = true;
    try {
      prefs = await getWarningPrefs();
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function toggle(warningType: string, action: 'blur' | 'show') {
    error = null;
    try {
      await setWarningPref({ warning_type: warningType, action });
      await load();
    } catch (failure) {
      error = failure;
    }
  }

  function getAction(warningType: string): 'blur' | 'show' | undefined {
    return prefs.find((p) => p.warning_type === warningType)?.action;
  }

  void load();
</script>

<div class="warning-prefs">
  <h3>Content Warnings</h3>
  <p class="lede">
    Choose how each type of warning is handled by default.
  </p>

  {#if loading}
    <p>Loading…</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th>Warning</th>
          <th>Blur by default</th>
          <th>Show by default</th>
        </tr>
      </thead>
      <tbody>
        {#each WARNING_TYPES as wt (wt.value)}
          <tr>
            <td>{wt.label}</td>
            <td>
              <input
                type="radio"
                checked={getAction(wt.value) === 'blur'}
                onclick={() => toggle(wt.value, 'blur')}
              />
            </td>
            <td>
              <input
                type="radio"
                checked={getAction(wt.value) === 'show'}
                onclick={() => toggle(wt.value, 'show')}
              />
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  {#if error}
    <p class="error">{error}</p>
  {/if}
</div>

<style>
  .warning-prefs {
    max-width: 40rem;
  }
  .lede {
    color: var(--text-muted, #666);
    margin-bottom: 1rem;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: 0.5rem;
    text-align: left;
    border-bottom: 1px solid var(--border, #ddd);
  }
  th {
    font-weight: 600;
  }
  .error {
    color: var(--error, #c00);
    font-size: 0.875rem;
  }
</style>