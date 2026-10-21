import path from 'path';
import fs from 'fs';
import { pathToFileURL } from 'url';

import { localized } from '../../intl/localize';
import { escapeHTML } from 'underscore.string';
import { App, PrintWindows } from '../../platform/app-api';
import {
  contentSecurityPolicyMetaTag,
  printContentSecurityPolicy,
} from '../../mail/content-security-policy';

interface PrintWindowOptions {
  subject: string;
  account: { name: string; email: string };
  participants: { name?: string; email: string }[];
  styleTags: string;
  htmlContent: string;
  // The messages' HTML as a JSON array.
  printMessages: string;
  remoteImages: boolean;
}

/**
 * Writes print.html and print.messages.js to the temp directory and opens them in the print
 * preview window main builds (main/windows/print-window.ts). print.js shows the print prompt
 * once the page has loaded; we can also call print directly from this process, but inside
 * print.js we can make sure to call window.print() after we've cleaned up the dom for printing.
 */
export default async function openPrintWindow({
  subject,
  account,
  participants,
  styleTags,
  htmlContent,
  printMessages,
  remoteImages,
}: PrintWindowOptions) {
  const tmp = await App.getPath('temp');
  const tmpMessagesPath = path.join(tmp, 'print.messages.js');

  const scriptPath = path.join(__dirname, 'static', 'print.js');
  const stylesPath = path.join(__dirname, 'static', 'styles.css');
  const participantsHtml = participants
    .map((part) => {
      return `<li class="participant"><span>
        ${escapeHTML(part.name || '')} &lt;${escapeHTML(part.email)}&gt;
      </span></li>`;
    })
    .join('');

  const content = `
    <!DOCTYPE html>
    <html>
      <head>
        ${contentSecurityPolicyMetaTag(printContentSecurityPolicy({ remoteImages }))}
        <meta charset="utf-8">
        ${styleTags}
        <link rel="stylesheet" type="text/css" href="${pathToFileURL(stylesPath).href}">
      </head>
      <body>
        <div id="print-header">
          <div id="print-note" style="display: none;">
            One or more messages in this thread were collapsed and will not be printed.
            To print these messages, expand them in the main window.
          </div>
          <div style="padding: 10px 14px;">
            <div id="close-button">
              ${escapeHTML(localized('Close'))}
            </div>
            <div id="print-button">
              ${escapeHTML(localized('Print'))}
            </div>
            <div id="print-pdf-button">
              ${escapeHTML(localized('Save as PDF'))}
            </div>
            <div class="logo-wrapper">
              <span class="account">${escapeHTML(account.name)} &lt;${escapeHTML(
                account.email
              )}&gt;</span>
            </div>
          </div>
        </div>
        <div id="print-header-spacing"></div>
        <h1 class="print-subject">${escapeHTML(subject)}</h1>
        <div class="print-participants">
          <ul>
            ${participantsHtml}
          </ul>
        </div>
        ${htmlContent}
        <script type="text/javascript" src="${pathToFileURL(tmpMessagesPath).href}"></script>
        <script type="text/javascript" src="${pathToFileURL(scriptPath).href}"></script>
      </body>
    </html>
  `;

  fs.writeFileSync(tmpMessagesPath, `window.printMessages = ${printMessages}`);
  // Main loads this file by its name in the temp directory.
  fs.writeFileSync(path.join(tmp, 'print.html'), content);
  await PrintWindows.open({ subject });
}
