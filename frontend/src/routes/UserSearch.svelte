<script lang="ts">
  import { searchUsers, type UserSearchResult } from '../lib/api';

  let query = '';
  let minWorks: number | null = null;
  let maxWorks: number | null = null;
  let fandom = '';
  let joinedAfter = '';

  let results: UserSearchResult[] | null = null;
  let loading = false;
  let error: string | null = null;
  /** So an empty page can say "nobody matched" instead of showing nothing. */
  let searched = false;

  /** Svelte hands a number input a number, an empty string, or null. */
  function asNumber(value: number | string | null): number | null {
    if (value === null || value === '') return null;
    const n = typeof value === 'number' ? value : Number(value);
    return Number.isFinite(n) ? n : null;
  }

  /** The range complaint, or null when the two ends can be sent. */
  function rangeProblem(): string | null {
    const lo = asNumber(minWorks);
    const hi = asNumber(maxWorks);
    if (lo !== null && hi !== null && lo > hi) {
      return 'The minimum cannot be greater than the maximum.';
    }
    return null;
  }

  /**
   * The work-count term, in the one spelling the backend prefers.
   *
   * Both ends become a single `lo..hi`, which is inclusive at both ends — the
   * same rule the word-count box on the works search uses. One end alone is a
   * `>=` or `<=`, because `works:10` means *exactly* ten and a reader who
   * types ten in a "minimum" box means "at least".
   */
  function worksTerm(): string | null {
    const lo = asNumber(minWorks);
    const hi = asNumber(maxWorks);
    if (lo !== null && hi !== null) return `works:${lo}..${hi}`;
    if (lo !== null) return `works:>=${lo}`;
    if (hi !== null) return `works:<=${hi}`;
    return null;
  }

  function buildQuery(): string {
    const terms: string[] = [];
    const free = query.trim();

    const works = worksTerm();
    if (works) terms.push(works);
    if (fandom.trim()) terms.push(`fandoms:"${fandom.trim()}"`);
    if (joinedAfter.trim()) terms.push(`joined:>${joinedAfter.trim()}`);

    // Free text is a term like any other, so it is conjoined rather than
    // searched separately. A reader who sets a filter *and* types a name means
    // both, and this is where that has to be true.
    if (free) terms.unshift(free);
    return terms.join(' AND ');
  }

  async function runSearch() {
    const problem = rangeProblem();
    if (problem) {
      error = problem;
      results = null;
      return;
    }
    const full = buildQuery();
    if (!full.trim()) {
      results = null;
      return;
    }
    loading = true;
    error = null;
    searched = true;
    try {
      const page = await searchUsers(full, 20);
      results = page.items;
    } catch (failure) {
      // The backend writes a 422 that says which surface a field belongs to.
      // Showing it verbatim is the difference between a reader who fixes the
      // query and a reader who thinks the search is broken.
      error = failure instanceof Error ? failure.message : String(failure);
      results = null;
    } finally {
      loading = false;
    }
  }
</script>

<div class="user-search">
  <h2>Find people</h2>

  <form on:submit|preventDefault={runSearch}>
    <div class="row">
      <label>
        Search pseudonyms
        <input type="search" bind:value={query} placeholder="a handle or display name" />
      </label>
    </div>

    <fieldset>
      <legend>Filters</legend>
      <div class="row">
        <label>
          Minimum works
          <input type="number" min="0" bind:value={minWorks} />
        </label>
        <label>
          Maximum works
          <input type="number" min="0" bind:value={maxWorks} />
        </label>
      </div>
      <div class="row">
        <label>
          Fandom
          <input type="text" bind:value={fandom} placeholder="Good Omens" />
        </label>
        <label>
          Joined after
          <input type="date" bind:value={joinedAfter} />
        </label>
      </div>
    </fieldset>

    <button type="submit" disabled={loading}>Search</button>
  </form>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if searched && results !== null && results.length === 0}
    <p class="empty">No pseudonyms matched that search.</p>
  {:else if results !== null && results.length > 0}
    <p class="count">
      {results.length}
      {results.length === 1 ? 'pseudonym' : 'pseudonyms'} found
    </p>
    <ul>
      {#each results as person (person.pseud_id)}
        <li>
          <a href={`/users/${person.handle}`}>{person.handle}</a>
          {#if person.display_name}
            <span class="display">— {person.display_name}</span>
          {/if}
          {#if person.joined_at}
            <span class="joined">joined {person.joined_at.slice(0, 10)}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .user-search {
    max-width: 44rem;
  }
  .row {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .row label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    flex: 1 1 12rem;
  }
  fieldset {
    border: 1px solid var(--rule, #ccc);
    margin: 1rem 0;
  }
  .error {
    color: var(--danger, #b00);
  }
  .count {
    font-weight: 600;
  }
  ul {
    list-style: none;
    padding: 0;
  }
  li {
    padding: 0.5rem 0;
    border-bottom: 1px solid var(--rule, #eee);
  }
  .display,
  .joined {
    opacity: 0.75;
    margin-left: 0.5rem;
  }
</style>
