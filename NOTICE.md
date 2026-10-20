# Notice

Tern is a fork of Mailspring and Mailspring-Sync, Copyright © 2014-2026 Foundry 376, LLC and
contributors. Tern received both under the GNU General Public License, version 3 or (at your
option) any later version, and distributes Tern, including the modifications made by the Tern
contributors, under the GNU General Public License, version 3 only (GPL-3.0-only). The licence
text is in LICENSE.md.

- `apps/desktop/` is derived from https://github.com/Foundry376/Mailspring (imported with history
  at upstream commit `1b52a4b19`) and has been modified since.
- `legacy/mailsync-cpp/` is https://github.com/Foundry376/Mailspring-Sync (imported with history
  at upstream commit `f7b0239`). It is kept as the reference for the Rust engine and is not
  shipped.
- `apps/engine/` and `crates/` are a Rust translation of Mailspring-Sync and are a derivative
  work of it. Parts of them are also translated from mailcore2, libetpan and iCalendarLib, whose
  notices follow.
- `apps/backend/` and `crates/ms-wire/` are written by the Tern contributors.

Each file keeps its upstream copyright notices. Changes since the import are recorded in the git
history of this repository.

Tern is not affiliated with or endorsed by Foundry 376 or the Mailspring project.

The Tern name and logos (the app icon, the bird mark and the tray icons, in `assets/brand/` and
copied into the desktop app) are trademarks of the Tern project, all rights reserved. They are
not covered by the GPL, and the desktop app's packages include them as its app, window,
notification and tray icons and on its welcome screen. A fork or other modified version that is
distributed must replace them and the name with its own; see `assets/brand/LICENSE.md`.

Third-party components that ship inside Tern's binaries are listed with their licences in
`THIRD-PARTY-NOTICES.txt` next to the engine and backend binaries, and in the desktop app under
View License in the application menu (`static/all_licenses.html`). The sections below cover code
that was translated or copied into Tern's own source files.

On Linux the engine links libdbus (https://gitlab.freedesktop.org/dbus/dbus) statically, from
the copy vendored in the `libdbus-sys` crate, to reach the Secret Service. libdbus is offered
under the Academic Free License 2.1 or the GNU General Public License, version 2 or any later
version; Tern uses it under the GPL. Its source is in the `libdbus-sys` crate (`vendor/dbus`).

## mailcore2

`crates/ms-imap`, `crates/ms-smtp`, `crates/ms-mime`, `crates/ms-core` and `crates/ms-providers`
translate parts of mailcore2 (https://github.com/MailCore/mailcore2) as modified in
Mailspring-Sync's `Vendor/mailcore2`.

```
MailCore 2

Copyright (C) 2001 - 2013 - MailCore team
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.
3. Neither the name of the MailCore project nor the names of its
   contributors may be used to endorse or promote products derived
   from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE AUTHORS AND CONTRIBUTORS ``AS IS'' AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHORS OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
SUCH DAMAGE.
```

The test corpus in `crates/ms-mime/tests/corpus/mailcore/` is taken from mailcore2's tests and
keeps its own LICENSE file. It is used by tests only and is not shipped.

## libEtPan!

`crates/ms-imap`, `crates/ms-smtp`, `crates/ms-mime` and `crates/ms-providers` translate parts
of libEtPan! (https://github.com/dinhvh/libetpan) as modified in Mailspring-Sync's
`Vendor/libetpan`: the IMAP, SMTP and IMF code (`mailimap*.c`, `mailsmtp*.c`, `mailimf.c`,
`mailstream_helper.c` and the CONDSTORE, QRESYNC, UIDPLUS, X-GM-LABELS and OAuth2 extensions).

```
libEtPan! -- a mail stuff library

Copyright (C) 2001 - 2005 - DINH Viet Hoa
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.
3. Neither the name of the libEtPan! project nor the names of its
   contributors may be used to endorse or promote products derived
   from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE AUTHORS AND CONTRIBUTORS ``AS IS'' AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHORS OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
SUCH DAMAGE.
```

libEtPan! as a whole also contains code from sendmail, NetBSD, the RSA Data Security MD5
Message-Digest Algorithm and Cyrus IMAP. None of those parts is translated into Tern.

## iCalendarLib

`crates/ms-dav` (`src/ical.rs`) translates parts of iCalendarLib by Juliusz Gonera
(https://github.com/jgonera/icalendarlib, 2010), as vendored in Mailspring-Sync's
`Vendor/icalendarlib`. Its README states that it is provided under the MIT licence. Upstream
carries no copyright line; the one below names the author from the repository history.

```
The MIT License

Copyright (c) 2010 Juliusz Gonera

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## AngularJS

`encodeHTMLEntities` in `apps/desktop/app/src/flux/models/utils.ts` is taken from AngularJS's
`ngSanitize` (https://github.com/angular/angular.js, `src/ngSanitize/sanitize.js`).

```
The MIT License

Copyright (c) 2010-2020 Google LLC. http://angularjs.org

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## slate-html-serializer

`TEXT_RULE_IMPROVED` in `apps/desktop/app/src/components/composer-editor/conversion.tsx` is
adapted from slate-html-serializer (https://github.com/ianstormtaylor/slate).

```
The MIT License

Copyright (c) 2016–2017, Ian Storm Taylor

Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## Dark Reader

The colour mapping in `packages/email-dark/src/color.ts` (`rgbToHsl`, `hslToRgb`, `scale`,
`darkBackground`, `lightForeground`, `darkBorder`) is ported from Dark Reader's dynamic mode
(https://github.com/darkreader/darkreader, `src/utils/color.ts`, `src/utils/math.ts` and
`src/inject/dynamic-theme/modify-colors.ts`).

```
MIT License

Copyright (c) 2026 Dark Reader Ltd.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## MailTrackerBlocker

The tracker rules in
`apps/desktop/app/features/remove-tracking-pixels/lib/mail-tracker-blocker-rules.ts` are
ported from MailTrackerBlocker (https://github.com/apparition47/MailTrackerBlocker,
`Source/MTBBlockedMessage.m`, commit `7463682a3`). The engine's draft pipeline keeps a copy of
the vendor rules in `crates/ms-compose/src/desktop_data.json` (`trackerRules`).

```
BSD 3-Clause License

Copyright (c) 2020, Aaron Lee.
All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

 * Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

 * Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

 * Neither the name MailTrackerBlocker nor the names of its contributors may be used to
   endorse or promote products derived from this software without specific
   prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Blink default stylesheet

`apps/desktop/app/src/chrome-user-agent-stylesheet-string.ts`, and its copy in
`crates/ms-compose/src/desktop_data.json` (`userAgentStylesheet`), are the default HTML
stylesheet of Chromium's Blink engine (`Source/core/css/html.css`), which comes from KHTML and
WebKit: Copyright (C) 2000 Lars Knoll, Copyright (C) 2003-2011 Apple Inc. The upstream file is
under the GNU Library General Public License, version 2 or (at your option) any later version.
