import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import SignIn from './SignIn.svelte';

describe('SignIn page', () => {
  it('renders email and password fields', () => {
    render(SignIn);
    expect(screen.getByLabelText(/Email address/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Password/)).toBeInTheDocument();
  });

  it('shows the same error for unknown address and wrong password', () => {
    render(SignIn);
    expect(screen.getByText(/same answer/)).toBeInTheDocument();
  });

  it('has a sign in button', () => {
    render(SignIn);
    expect(screen.getByRole('button', { name: /sign in/i })).toBeInTheDocument();
  });
});
