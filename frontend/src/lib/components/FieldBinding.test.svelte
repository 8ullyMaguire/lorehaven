<script lang="ts">
  /**
   * A harness for the field primitives' binding contract.
   *
   * It exists so that `bind:value` on `TextField`, `Textarea` and `Select` can
   * be tested as a *component* binding — the thing a page actually writes —
   * rather than as a DOM event. The bug this pins: without `$bindable()`, the
   * compiler accepts the binding, the runtime drops it, and the parent submits
   * an empty field with no error anywhere.
   */
  import Select from './Select.svelte';
  import Textarea from './Textarea.svelte';
  import TextField from './TextField.svelte';

  interface Props {
    onvalues?: (values: { text: string; area: string; choice: string }) => void;
    /** An explicit handler alongside the binding, as the shell uses. */
    onselect?: (value: string) => void;
  }

  let { onvalues, onselect }: Props = $props();

  let text = $state('');
  let area = $state('');
  let choice = $state('a');

  $effect(() => {
    onvalues?.({ text, area, choice });
  });
</script>

<div>
  <TextField label="Text" bind:value={text} />
  <Textarea label="Area" bind:value={area} />
  <Select
    label="Choice"
    bind:value={choice}
    onchange={(event) => onselect?.(event.currentTarget.value)}
    options={[
      { value: 'a', label: 'A' },
      { value: 'b', label: 'B' },
    ]}
  />
</div>
