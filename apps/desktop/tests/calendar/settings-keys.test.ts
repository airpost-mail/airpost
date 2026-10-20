import { MainCalendar } from '../../src/features/main-calendar/main-calendar';
import { CalendarView } from '../../src/features/main-calendar/constants';

describe('the calendar settings', function () {
  it('reads the view and the hidden calendars from core.calendar', function () {
    AppEnv.config.set('core.calendar.view', CalendarView.MONTH);
    AppEnv.config.set('core.calendar.disabledCalendars', ['cal-off']);
    const cal: any = new MainCalendar({} as any);
    expect(cal.state.view).toBe(CalendarView.MONTH);
    expect(cal.state.disabledCalendars).toEqual(['cal-off']);
  });

  it('does not read the keys Mailspring keeps them under', function () {
    AppEnv.config.set('mailspring.calendarView', CalendarView.MONTH);
    AppEnv.config.set('mailspring.disabledCalendars', ['cal-off']);
    const cal: any = new MainCalendar({} as any);
    expect(cal.state.view).toBe(CalendarView.WEEK);
    expect(cal.state.disabledCalendars).toEqual([]);
  });

  it('saves a view change under core.calendar.view', function () {
    const cal: any = new MainCalendar({} as any);
    spyOn(cal, 'setState');
    cal.onChangeView(CalendarView.DAY);
    expect(AppEnv.config.get('core.calendar.view')).toBe(CalendarView.DAY);
    expect(AppEnv.config.get('mailspring.calendarView')).toBeUndefined();
  });
});
