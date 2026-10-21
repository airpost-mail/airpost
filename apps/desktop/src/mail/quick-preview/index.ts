import { execFile } from 'child_process';
import path from 'path';
import { File } from '../../data/models/file';
import { CurrentWindow, QuickPreviewWindows } from '../../platform/app-api';

// The preview and thumbnail windows are main's (main/windows/quick-preview.ts); this module
// picks the strategy, and runs Quick Look on macOS.

const FileSizeLimit = 5 * 1024 * 1024;
const ThumbnailWidth = 320 * (11 / 8.5);
// Budget for generating a single thumbnail, shared by both strategies.
const PreviewTimeout = 5000;
const QuicklookIsAvailable = process.platform === 'darwin';

const QuicklookBlacklist = [
  'jpg',
  'bmp',
  'gif',
  'png',
  'jpeg',
  'zip',
  'tar',
  'gz',
  'bz2',
  'dmg',
  'exe',
  'ics',
];

const CrossplatformStrategies: { [strategy: string]: string[] } = {
  pdfjs: ['pdf'],
  mammoth: ['docx'],
  snarkdown: ['md'],
  xlsx: ['xls', 'xlsx', 'csv', 'eth', 'ods', 'fods', 'uos1', 'uos2', 'dbf', 'prn', 'xlw', 'xlsb'],
  prism: [
    'html',
    'svg',
    'xml',
    'css',
    'c',
    'cc',
    'cpp',
    'js',
    'jsx',
    'tsx',
    'ts',
    'go',
    'cs',
    'patch',
    'swift',
    'java',
    'json',
    'jsonp',
    'tex',
    'mm',
    'm',
    'h',
    'py',
    'rb',
    'rs',
    'sql',
    'yml',
    'yaml',
    'txt',
    'log',
  ],
};

const CrossplatformStrategiesBetterThanQuicklook = ['snarkdown', 'prism'];

function strategyForPreviewing(ext: string) {
  if (ext.startsWith('.')) ext = ext.substr(1);

  const strategy = Object.keys(CrossplatformStrategies).find((strategy) =>
    CrossplatformStrategies[strategy].includes(ext)
  );

  if (QuicklookIsAvailable && !QuicklookBlacklist.includes(ext)) {
    if (!strategy || !CrossplatformStrategiesBetterThanQuicklook.includes(strategy)) {
      return 'quicklook';
    }
  }

  return strategy;
}

export function canPossiblyPreviewExtension(file: File) {
  if (file.size > FileSizeLimit) {
    return false;
  }
  return !!strategyForPreviewing(file.displayExtension());
}

export function displayQuickPreviewWindow(filePath: string) {
  const strategy = strategyForPreviewing(path.extname(filePath));

  if (strategy === 'quicklook') {
    CurrentWindow.previewFile(filePath);
    return;
  }
  QuickPreviewWindows.display({ filePath, strategy }).catch((err: Error) => {
    console.error('Quickpreview display window failed:', err);
  });
}

export async function generatePreview({
  file,
  filePath,
  previewPath,
}: {
  file: File;
  filePath: string;
  previewPath: string;
}) {
  const strategy = strategyForPreviewing(file.displayExtension());

  if (strategy === 'quicklook') {
    return await _generateQuicklookPreview({ filePath });
  } else if (strategy) {
    return await _generateCrossplatformPreview({ filePath, previewPath, strategy });
  } else {
    return false;
  }
}

// Private

async function _generateCrossplatformPreview({
  filePath,
  previewPath,
  strategy,
}: {
  filePath: string;
  previewPath: string;
  strategy: string;
}) {
  try {
    return await QuickPreviewWindows.capture({ filePath, previewPath, strategy });
  } catch (err) {
    console.error('Quickpreview capture failed:', err);
    return false;
  }
}

async function _generateQuicklookPreview({ filePath }: { filePath: string }) {
  const dirQuoted = path.dirname(filePath).replace(/"/g, '\\"');
  const pathQuoted = filePath.replace(/"/g, '\\"');

  return new Promise<boolean>((resolve) => {
    const cmd = '/usr/bin/qlmanage';
    const args = [
      '-t',
      '-f',
      `${window.devicePixelRatio}`,
      '-s',
      `${ThumbnailWidth}`,
      '-o',
      dirQuoted,
      pathQuoted,
    ];

    // qlmanage can hang indefinitely rather than failing -- an attachment saved
    // without an extension is enough to do it, and Tern writes those out
    // as "noname". Without a timeout the callback never fires and the child is
    // never reaped, so each one leaks holding a pkd connection open; enough of
    // them exhausts pkd's thread pool and every app that builds a Share menu
    // then deadlocks on a synchronous XPC call to it.
    // SIGKILL rather than the default SIGTERM: qlmanage handles SIGTERM and
    // exits 0, so the timeout would look like a success and we would report a
    // thumbnail that was never written.
    execFile(
      cmd,
      args,
      { timeout: PreviewTimeout, killSignal: 'SIGKILL' },
      (error, stdout, stderr) => {
        // Note: sometimes qlmanage outputs to stderr but still successfully
        // produces a thumbnail. It complains about bad plugins pretty often.
        if (
          error ||
          stdout.match(/No thumbnail created/i) ||
          (stderr && !stdout.includes('produced one thumbnail'))
        ) {
          resolve(false);
        } else {
          resolve(true);
        }
      }
    );
  });
}
