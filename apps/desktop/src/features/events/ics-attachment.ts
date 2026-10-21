import { File } from '../../data/models/file';

export function bestICSAttachment(files: File[]) {
  return (
    files.find((f) => f.filename.endsWith('.ics')) ||
    files.find((f) => f.contentType === 'text/calendar') ||
    files.find((f) => f.filename.endsWith('.vcs'))
  );
}
