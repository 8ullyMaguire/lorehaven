import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import Search from './Search.svelte';

describe('Search page', () => {
  it('renders search input and filter options', () => {
    render(Search);
    expect(
      screen.getByPlaceholderText('e.g. fandom:"Harry Potter" AND tag:draco'),
    ).toBeInTheDocument();
    expect(screen.getByText('Field')).toBeInTheDocument();
  });

  it('shows the empty state before any search', () => {
    render(Search);
    expect(screen.getByText(/search works/i)).toBeInTheDocument();
  });

  it('renders filter field options', () => {
    render(Search);
    expect(screen.getByText('Fandom')).toBeInTheDocument();
    expect(screen.getByText('Tag')).toBeInTheDocument();
    expect(screen.getByText('Rating')).toBeInTheDocument();
  });
});
