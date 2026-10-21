// In packaged builds we don't ship the TypeScript compiler, so trying to load
// a plugin's raw .ts/.tsx source can't work. Register tiny require.extensions
// hooks that surface a clear "must be compiled" dialog instead of letting Node
// fail with a syntax error on the type annotations or JSX.

let dialogShown = false;

function showDialog(filePath) {
  if (dialogShown) return;
  dialogShown = true;
  const title = 'Plugin must be compiled';
  const content =
    `Tern does not ship with a TypeScript compiler to recompile plugins on the fly. ` +
    `Ask the plugin developer to compile the plugin to vanilla JavaScript as a pre-publish step.` +
    `\n\nFile: ${filePath}`;
  if (process.type === 'renderer') {
    // A window asks main (platform/app-api-channels.ts) and doesn't wait for the dialog; the
    // hook throws right after.
    require('electron').ipcRenderer.invoke('app-api:dialog:show-error-box', title, content);
  } else {
    require('electron').dialog.showErrorBox(title, content);
  }
}

['.ts', '.tsx', '.jsx', '.es6'].forEach(extension => {
  Object.defineProperty(require.extensions, extension, {
    enumerable: true,
    writable: true,
    value: (module, filePath) => {
      showDialog(filePath);
      throw new Error(
        `Cannot load ${filePath}: Tern does not ship with a TypeScript compiler. ` +
          `Plugins must be compiled to vanilla JavaScript as a pre-publish step.`
      );
    },
  });
});
