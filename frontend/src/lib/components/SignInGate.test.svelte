<script lang="ts">
  /**
   * A harness for `SignInGate`'s three states.
   *
   * The gate renders a snippet, and a snippet cannot be constructed in a test
   * without one — which is the same reason `FieldBinding.test.svelte` exists for
   * the field primitives. So this file supplies the page content the gate wraps, and
   * `SignInGate.test.ts` renders THIS to reach each branch.
   *
   * The child marks itself with `data-testid="page-body"`, because the assertion
   * that matters in each state is "the page is/is not there" — and that is
   * invisible without something distinguishable to look for.
   */
  import SignInGate from './SignInGate.svelte';

  interface Props {
    purpose: string;
    skeletonLines?: number;
  }

  let { purpose, skeletonLines = 4 }: Props = $props();
</script>

<SignInGate {purpose} {skeletonLines}>
  <p data-testid="page-body">The page itself.</p>
</SignInGate>
