import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Combobox from './Combobox.svelte';

describe('Combobox', () => {
	const options = [
		{ value: 'apple', label: 'Apple' },
		{ value: 'banana', label: 'Banana' },
		{ value: 'cherry', label: 'Cherry', disabled: true }
	];

	it('renders an editable combobox linked to a datalist of options', () => {
		render(Combobox, {
			props: { label: 'Fruit', options, value: '', onchange: vi.fn() }
		});

		const input = screen.getByRole('combobox');
		expect(input).toBeInTheDocument();
		// input has list attribute pointing to the datalist
		expect(input).toHaveAttribute('list', 'Fruit');
		expect(input).toHaveAttribute('aria-autocomplete', 'list');
		expect(input).toHaveAttribute('aria-expanded', 'false');
		expect(input).toHaveAttribute('aria-controls', 'Fruit');
	});

	it('updates value and calls onchange when the user types', async () => {
		const onchange = vi.fn();
		render(Combobox, {
			props: { label: 'Fruit', options, value: '', onchange }
		});
		const input = screen.getByRole('combobox') as HTMLInputElement;

		await fireEvent.input(input, { target: { value: 'apple' } });
		expect(onchange).toHaveBeenCalledWith('apple');

		await fireEvent.input(input, { target: { value: 'banana' } });
		expect(onchange).toHaveBeenCalledWith('banana');
	});

	it('shows hint text when provided', () => {
		render(Combobox, {
			props: { label: 'Fruit', options, value: '', onchange: vi.fn(), hint: 'Select a fruit' }
		});
		expect(screen.getByText('Select a fruit')).toBeInTheDocument();
	});

	it('shows error text when provided', () => {
		render(Combobox, {
			props: { label: 'Fruit', options, value: 'apple', onchange: vi.fn(), error: 'Invalid choice' }
		});
		expect(screen.getByText('Invalid choice')).toBeInTheDocument();
	});

	it('combines hint and error in aria-describedby when both are present', () => {
		render(Combobox, {
			props: { label: 'Fruit', options, value: 'apple', onchange: vi.fn(), hint: 'Select a fruit', error: 'Invalid choice' }
		});
		const input = screen.getByRole('combobox');
		// When both are present, both IDs should be in aria-describedby
		const describedBy = input.getAttribute('aria-describedby');
		expect(describedBy).toContain('Fruit-helptext');
		expect(describedBy).toContain('Fruit-error');
	});
});