import { EventEmitter } from 'events';
import https from 'https';
import { shell } from 'electron';
import url from 'url';

function safeHttpUrl(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return null;
    return parsed.href;
  } catch {
    return null;
  }
}

// The update server's answer to a check; url is checked to be http(s) before it is passed on.
export interface UpdateJSON {
  url: string;
  version?: string;
}

export default class AutoupdateImplBase extends EventEmitter {
  feedURL = '';
  lastRetrievedUpdateURL: string | null = null;

  supportsUpdates() {
    // If we're packaged into a Snapcraft distribution, we don't need
    // autoupdates within the app because they're handled transparently.
    if (process.env.SNAP) {
      return false;
    }
    return true;
  }

  /* Public: Set the feed URL where we retrieve update information. */
  setFeedURL(feedURL: string) {
    this.feedURL = feedURL;
    this.lastRetrievedUpdateURL = null;
  }

  emitError = (error: unknown) => {
    if (this.listenerCount('error') > 0) {
      this.emit('error', error);
    } else {
      console.error(
        'Autoupdater error (unhandled):',
        error instanceof Error ? error.message : String(error)
      );
    }
  };

  manuallyQueryUpdateServer(successCallback: (json: UpdateJSON | false) => void) {
    const feedHost = url.parse(this.feedURL).hostname;
    if (!feedHost) {
      this.emitError(new Error(`Autoupdater feed URL has no host: ${this.feedURL}`));
      return;
    }
    const feedPath = this.feedURL.split(feedHost).pop();

    // Hit the feed URL ourselves and see if an update is available.
    // On linux we can't autoupdate, but we can still show the "update available" bar.
    https
      .get({ host: feedHost, path: feedPath }, (res) => {
        console.log(`Manual update check (${feedHost}${feedPath}) returned ${res.statusCode}`);

        if (res.statusCode === 204) {
          successCallback(false);
          return;
        }

        if (res.statusCode !== 200) {
          this.emitError(new Error(`Autoupdater server returned status ${res.statusCode}`));
          res.resume(); // drain the response so the socket can be freed
          return;
        }

        let data = '';
        res.on('error', this.emitError);
        res.on('data', (chunk) => {
          data += chunk;
        });
        res.on('end', () => {
          try {
            const json = JSON.parse(data);
            if (!json.url) {
              this.emitError(new Error(`Autoupdater response did not include URL: ${data}`));
              return;
            }
            const safeUrl = safeHttpUrl(json.url);
            if (!safeUrl) {
              this.emitError(
                new Error(`Autoupdater response URL has disallowed scheme: ${json.url}`)
              );
              return;
            }
            json.url = safeUrl;
            successCallback(json);
          } catch (err) {
            this.emitError(err);
          }
        });
      })
      .on('error', this.emitError);
  }

  /* Public: Check for updates and emit events if an update is available. */
  checkForUpdates() {
    if (!this.feedURL) {
      return;
    }

    this.emit('checking-for-update');

    this.manuallyQueryUpdateServer((json) => {
      if (!json) {
        this.emit('update-not-available');
        return;
      }
      this.lastRetrievedUpdateURL = json.url;
      this.emit('update-downloaded', null, 'manual-download', json.version);
    });
  }

  /* Public: Install the update. */
  quitAndInstall() {
    const downloadURL = safeHttpUrl(this.lastRetrievedUpdateURL);
    if (downloadURL) shell.openExternal(downloadURL);
  }
}
