// Owned by the calendar feature.
import { createFileRoute } from '@tanstack/react-router';
import { CalendarView } from '@/features/calendar';

export const Route = createFileRoute('/_app/calendar')({
  component: CalendarView,
});
