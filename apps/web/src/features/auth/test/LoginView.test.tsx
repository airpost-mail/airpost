import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { LoginView } from '../LoginView';

function renderLogin() {
  const root = createRootRoute({ component: () => <LoginView /> });
  const router = createRouter({
    routeTree: root,
    history: createMemoryHistory({ initialEntries: ['/'] }),
  });
  render(
    <QueryClientProvider client={new QueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
}

describe('LoginView', () => {
  it('shows the server it will sign in to and the demo hint', async () => {
    renderLogin();
    expect(await screen.findByText(window.location.host)).toBeInTheDocument();
    expect(await screen.findByText(/any credentials work/)).toBeInTheDocument();
    expect(screen.getByText(/runs a demo engine in this tab/)).toBeInTheDocument();
  });

  it('validates a changed server address before using it', async () => {
    const user = userEvent.setup();
    renderLogin();
    await user.click(await screen.findByRole('button', { name: 'Change' }));
    const input = screen.getByLabelText('Server address');
    await user.clear(input);
    await user.type(input, 'http://tern.example.com{Enter}');
    expect(screen.getByText(/Use https:\/\/ unless/)).toBeInTheDocument();

    await user.clear(input);
    await user.type(input, 'tern.example.com{Enter}');
    expect(await screen.findByText('tern.example.com')).toBeInTheDocument();
  });

  it('offers a device token instead of a password', async () => {
    const user = userEvent.setup();
    renderLogin();
    await user.click(await screen.findByRole('tab', { name: 'Device token' }));
    expect(screen.getByLabelText('Device token', { selector: 'input' })).toHaveAttribute(
      'type',
      'password',
    );
    expect(screen.queryByLabelText('Email')).toBeNull();
  });
});
