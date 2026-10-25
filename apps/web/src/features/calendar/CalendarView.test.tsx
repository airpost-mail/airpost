import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { EngineProvider, MockEngine } from '@/engine';
import { CalendarView } from './CalendarView';

function setup(features?: string[]) {
  const engine = new MockEngine({ latency: 0, features });
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <EngineProvider client={engine}>
        <CalendarView />
      </EngineProvider>
    </QueryClientProvider>,
  );
  return engine;
}

describe('CalendarView', () => {
  it('lists the next two weeks of every calendar, and hides one', async () => {
    setup();
    const agenda = await screen.findByRole('list', { name: 'Agenda' });
    await waitFor(() => expect(within(agenda).getByText('Lunch with Ines')).toBeInTheDocument());
    expect(within(agenda).getByText('Quarterly planning')).toBeInTheDocument();
    expect(within(agenda).getByText("Marta's birthday")).toBeInTheDocument();
    // Two days ago is outside the range.
    expect(within(agenda).queryByText('Sprint review')).toBeNull();

    const calendars = screen.getByRole('complementary', { name: 'Calendars' });
    await userEvent.click(within(calendars).getByRole('checkbox', { name: /Work/ }));
    await waitFor(() => expect(within(agenda).queryByText('Quarterly planning')).toBeNull());
    expect(within(agenda).getByText('Lunch with Ines')).toBeInTheDocument();
  });

  it('pages back to earlier events', async () => {
    setup();
    await screen.findByRole('list', { name: 'Agenda' });
    await userEvent.click(screen.getByRole('button', { name: 'Previous two weeks' }));
    await waitFor(() => expect(screen.getByText('Sprint review')).toBeInTheDocument());
  });
});
