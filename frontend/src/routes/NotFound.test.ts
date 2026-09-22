import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import NotFound from './NotFound.svelte';

describe('NotFound page', () => {
  it('renders the missing path in the message', () => {
    render(NotFound, { props: { path: '/missing-page' } });
    expect(screen.getByText('No such page')).toBeInTheDocument();
    expect(screen.getByText('/missing-page')).toBeInTheDocument();
  });

  it('links back to the entrance', () => {
    render(NotFound, { props: { path: '/missing' } });
    const link = screen.getByRole('link', { name: 'Return to the entrance' });
    expect(link).toHaveAttribute('href', '/');
  });

  it('does not confirm whether a withdrawn work existed', () => {
    render(NotFound, { props: { path: '/works/xyz' } });
    expect(
      screen.getByText(/indistinguishable from one that never existed/),
    ).toBeInTheDocument();
  });
});
