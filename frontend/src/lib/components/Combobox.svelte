<script lang="ts">
	import type { HTMLInputEvent } from 'svelte/elements';

	export interface ComboboxOption {
		value: string;
		label: string;
		disabled?: boolean;
	}

	interface Props {
		label: string;
		options: ComboboxOption[];
		/** Bindable; see the note in TextField.svelte. */
		value: string;
		onchange: (value: string) => void;
		hint?: string;
		error?: string;
		placeholder?: string;
	}

	let {
		label,
		options,
		value,
		onchange,
		hint,
		error,
		placeholder = ''
	}: Props = $props();

	function handleInput(event: HTMLInputEvent) {
		const input = event.target as HTMLInputElement;
		onchange(input.value);
	}

	function getDescribedBy(): string | undefined {
		const ids: string[] = [];
		if (hint) ids.push(`${label}-helptext`);
		if (error) ids.push(`${label}-error`);
		return ids.length > 0 ? ids.join(' ') : undefined;
	}
</script>

<div class="field">
	<label for={label}>{label}</label>
	<div class="combobox">
		<input
			type="text"
			list={label}
			bind:value
			oninput={handleInput}
			placeholder={placeholder}
			aria-invalid={error ? 'true' : undefined}
			aria-describedby={getDescribedBy()}
			aria-expanded="false"
			aria-controls={label}
			aria-autocomplete="list"
			tabindex="0"
			id={label}
		/>
		<datalist id={`${label}-list`}>
			{#each options as option (option.value)}
				<option value={option.value} label={option.label} disabled={option.disabled}></option>
			{/each}
		</datalist>
	</div>
	{#if hint}
		<p class="hint" id={`${label}-helptext`}>{hint}</p>
	{/if}
	{#if error}
		<p class="error" id={`${label}-error`} role="alert">{error}</p>
	{/if}
</div>

<style>
	.field {
		display: flex;
		flex-direction: column;
		gap: var(--space-2);
		margin-bottom: var(--space-4);
	}

	label {
		font-size: var(--text-sm);
		font-weight: 600;
	}

	.combobox {
		position: relative;
		display: flex;
		gap: var(--space-1);
	}

	input {
		flex: 1;
		min-height: 2.75rem;
		padding: var(--space-3);
		font: inherit;
		color: var(--color-text);
		background: var(--color-surface);
		border: var(--border-width) solid var(--color-border-strong);
		border-radius: var(--radius-md);
	}

	input:focus {
		box-shadow: 0 0 0 2px var(--color-ring);
		border-color: var(--color-border-focused);
	}

	datalist {
		display: block;
	}

	option {
		padding: var(--space-2) var(--space-3);
	}

	option:disabled {
		color: var(--color-text-muted);
	}

	.hint {
		font-size: var(--text-xs);
		color: var(--color-text-muted);
	}

	.error {
		font-size: var(--text-xs);
		color: var(--color-text-error);
	}
</style>