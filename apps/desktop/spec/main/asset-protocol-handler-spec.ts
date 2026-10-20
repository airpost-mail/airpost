describe('"mailspring" protocol URL', () => {
  function statusOf(url: string) {
    let status: number | null = null;
    const request = new XMLHttpRequest();
    request.addEventListener('load', () => {
      status = request.status;
    });
    request.addEventListener('error', () => {
      status = -1;
    });
    request.open('GET', url, true);
    request.send();
    waitsFor(() => status !== null, `request to ${url} to be done`);
    return () => status;
  }

  // Built-in features live in src/features/<dir>; the host is still the directory name.
  it('serves files of a built-in feature', () => {
    const icon = statusOf('mailspring://account-sidebar/assets/icon-sidebar-addcategory@2x.png');
    const font = statusOf('mailspring://custom-fonts/fonts/Inter-Regular.woff2');
    runs(() => {
      expect(icon()).toBe(200);
      expect(font()).toBe(200);
    });
  });

  it('serves files of a bundled plugin', () => {
    const icon = statusOf('mailspring://translation/assets/icon-composer-translate@2x.png');
    runs(() => expect(icon()).toBe(200));
  });

  it('answers 404 for a file no root has', () => {
    const missing = statusOf('mailspring://account-sidebar/package.json');
    runs(() => expect(missing()).toBe(404));
  });
});
