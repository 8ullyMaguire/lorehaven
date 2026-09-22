import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import Docs from './Docs.svelte';

describe('Docs page', () => {
  it('renders the help index when no slug is given', () => {
    render(Docs);
    expect(screen.getByText('Help')).toBeInTheDocument();
  });

  it('shows the list of available docs', () => {
    render(Docs);
    expect(screen.getAllByText('Getting started').length).toBeGreaterThan(0);
  });

  it('renders a specific doc when a slug is given', () => {
    render(Docs, { props: { slug: 'getting-started' } });
    expect(screen.getByText('Help')).toBeInTheDocument();
    expect(screen.getAllByText('Getting started').length).toBeGreaterThan(0);
  });

  it('shows a 404 message for unknown slug', () => {
    render(Docs, { props: { slug: 'nonexistent' } });
    expect(screen.getByText('Page not found')).toBeInTheDocument();
  });
});
