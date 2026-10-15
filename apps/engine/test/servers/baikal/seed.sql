-- The harness user (test / pass) as Baikal's admin UI creates it: digesta1 is
-- md5("test:BaikalDAV:pass"), which PDOBasicAuth checks Basic credentials against, plus the
-- principal, a "default" calendar and a "default" address book.
INSERT INTO users (username, digesta1) VALUES ('test', '26b832ec57222d78f5d680aee2c4ee8a');
INSERT INTO principals (uri, email, displayname) VALUES ('principals/test', 'test@example.test', 'Harness Test');
INSERT INTO calendars (synctoken, components) VALUES (1, 'VEVENT,VTODO');
INSERT INTO calendarinstances (calendarid, principaluri, access, displayname, uri, description, calendarorder, calendarcolor, timezone, transparent, share_invitestatus)
  VALUES (1, 'principals/test', 1, 'Default calendar', 'default', 'Harness calendar', 0, '#4285f4', '', 0, 2);
INSERT INTO addressbooks (principaluri, displayname, uri, description, synctoken)
  VALUES ('principals/test', 'Default Address Book', 'default', 'Harness address book', 1);
