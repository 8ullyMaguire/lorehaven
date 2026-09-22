import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import Planned from './Planned.svelte';
import type { PlannedRoute } from '../lib/router';

const ROUTE: PlannedRoute = {
  title: 'Feed',
  summary: 'Your personalized feed',
  milestone: 'M45',
};

describe('Planned page', () => {
  it('renders the planned route title', () => {
    render(Planned, { props: { route: ROUTE } });
    expect(screen.getByText('Feed')).toBeInTheDocument();
  });

  it('renders the summary', () => {
    render(Planned, { props: { route: ROUTE } });
    expect(screen.getByText('Your personalized feed')).toBeInTheDocument();
  });

  it('shows a "Not built yet" badge', () => {
    render(Planned, { props: { route: ROUTE } });
    expect(screen.getByText('Not built yet')).toBeInTheDocument();
  });

  it('displays the milestone reference', () => {
    render(Planned, { props: { route: ROUTE } });
    expect(screen.getByText('M45')).toBeInTheDocument();
  });
});
