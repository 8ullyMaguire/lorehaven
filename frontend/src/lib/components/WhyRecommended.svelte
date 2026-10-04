<!--
  Item 9: why a recommendation appears.

  ## The server already knew this and told nobody

  `GET /discovery/slots/{slot_id}/explanation` has existed for a long time, and
  `crates/app/src/routes/discovery.rs` already writes a `slot_id` onto every feed item it
  serves. The data was recorded, the route answered it, and no element in the frontend
  called either. So this component is the whole of item 9 — there is no store work and no
  migration, which is also why this file is mostly about failure modes.

  ## Why this is a disclosure and not a tooltip

  A tooltip is invisible until hovered, which makes "why am I seeing this" something the
  reader has to know to ask about. A disclosure is one click and answers the question for
  everyone who has it. That matters here because the reader who most wants to know WHY a
  thing was recommended is the one being recommended the most, and a tooltip is exactly the
  affordance that hides from them.

  ## It asks for its own data, and only on request

  The explanation is a separate fetch rather than something the feed inlines. Fetching it
  for every item on every paint would be N requests for information most readers never ask
  for, and it would make the feed's own latency depend on a route that reads the recorded
  slots table. Asking on click means zero cost until there is a question.

  ## Every unavailable state renders NOTHING, not a disabled control

  Four ways this can have no explanation, all normal:

    - no `slot_id` on the item, because the server's `record_response` write failed. It
      warns-and-continues in that case rather than failing the feed, which is the right
      call for the feed and the reason the id is optional in the type;
    - a 404, which per §3.3 means either "no such slot" OR "not yours" and the door must
      not distinguish them, so this component cannot either;
    - a signed-out reader, where the route is `RequireSession`;
    - a request still in flight.

  A greyed-out "why?" button is the tempting choice and it is wrong in all four: it
  advertises a capability that cannot work, and it costs the reader a click to discover
  that. So the trigger is not in the DOM at all unless there is something behind it.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { fetchSlotExplanation, slotReasonLabel, type SlotExplanation } from '../api';

  interface Props {
    /** The recorded slot id, or absent when the server did not record one. */
    slotId?: string | undefined;
  }

  let { slotId }: Props = $props();

  let open = $state(false);
  let loading = $state(false);
  let explanation: SlotExplanation | null = $state(null);
  /** Set once a fetch has come back empty, so the trigger stops being offered. */
  let unavailable = $state(false);
  let controller: AbortController | null = null;

  /**
   * The trigger exists only when there is a slot to ask about AND a session to ask with.
   * Counted by the tests before any "renders nothing" assertion, because an absence
   * assertion against an empty component is vacuously true.
   */
  const available = $derived(Boolean(slotId) && !unavailable);

  async function toggle() {
    if (open) {
      open = false;
      return;
    }
    if (!slotId || loading) return;
    if (explanation) {
      // Already fetched: answer from memory rather than re-requesting a recording that
      // cannot change. The slot is immutable once written, so this is not a cache-age
      // question.
      open = true;
      return;
    }
    open = true;
    loading = true;
    controller = new AbortController();
    try {
      const result = await fetchSlotExplanation(slotId, controller.signal);
      if (result === null) {
        // 404: not this reader's slot, or no such slot. Either way there is no explanation
        // and the trigger retires itself.
        unavailable = true;
        open = false;
        return;
      }
      explanation = result;
    } catch (e) {
      // A network failure is NOT the same as "no explanation" — the slot may well exist.
      // Leave `unavailable` false so the reader can try again, and say so.
      unavailable = false;
    } finally {
      loading = false;
      controller = null;
    }
  }

  onDestroy(() => controller?.abort());
</script>

{#if available}
  <button
    type="button"
    class="why"
    data-testid="why-trigger"
    aria-expanded={open}
    onclick={toggle}
  >
    Why this?
  </button>
{/if}

{#if open && explanation}
  <ul class="reasons" data-testid="why-reasons" aria-label="Why you are seeing this">
    {#each explanation.reasons as reason}
      <li>{slotReasonLabel(reason)}</li>
    {/each}
  </ul>
{:else if open && loading}
  <p class="reasons" data-testid="why-loading">Checking why this appeared…</p>
{:else if open && explanation === null && !loading}
  <!-- The fetch failed rather than 404'd. Retry, because the slot probably exists. -->
  <button type="button" class="retry" data-testid="why-retry" onclick={toggle}>
    Could not load the reason. Try again.
  </button>
{/if}

<style>
  .why,
  .retry {
    background: none;
    border: 1px solid currentColor;
    border-radius: 3px;
    padding: 0.1rem 0.4rem;
    font: inherit;
    font-size: 0.8rem;
    opacity: 0.75;
    cursor: pointer;
  }

  .why:hover,
  .retry:hover {
    opacity: 1;
  }

  .reasons {
    margin: 0.35rem 0 0;
    padding-left: 1.1rem;
    font-size: 0.85rem;
    opacity: 0.85;
  }
</style>