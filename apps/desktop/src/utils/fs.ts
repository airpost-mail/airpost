import fs from 'fs';
import { randomUUID } from 'crypto';

export function atomicWriteFileSync(filepath: string, content: string) {
  const backupPath = `${filepath}.${randomUUID()}.bak`;
  fs.writeFileSync(backupPath, content);
  fs.renameSync(backupPath, filepath);
}
