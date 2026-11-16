// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

/*
Chromium has no AIFF demuxer, so the macOS system sounds (/System/Library/Sounds, 24-bit
big-endian PCM AIFF) are rewritten as WAV, which it plays. Only uncompressed AIFF is handled;
AIFF-C and anything malformed return null.
*/

// The 80-bit IEEE 754 extended float AIFF stores the sample rate in.
function readExtended(buf: Buffer, at: number) {
  const exponent = buf.readUInt16BE(at) & 0x7fff;
  const mantissa = buf.readUInt32BE(at + 2) * 2 ** 32 + buf.readUInt32BE(at + 6);
  return mantissa * 2 ** (exponent - 16383 - 63);
}

export function aiffToWav(aiff: Buffer): Buffer | null {
  if (aiff.length < 12 || aiff.toString('ascii', 0, 4) !== 'FORM') return null;
  if (aiff.toString('ascii', 8, 12) !== 'AIFF') return null;

  let channels = 0;
  let bits = 0;
  let rate = 0;
  let pcm: Buffer | null = null;
  for (let at = 12; at + 8 <= aiff.length; ) {
    const id = aiff.toString('ascii', at, at + 4);
    const size = aiff.readUInt32BE(at + 4);
    const body = at + 8;
    if (body + size > aiff.length) return null;
    if (id === 'COMM' && size >= 18) {
      channels = aiff.readUInt16BE(body);
      bits = aiff.readUInt16BE(body + 6);
      rate = Math.round(readExtended(aiff, body + 8));
    } else if (id === 'SSND' && size >= 8) {
      pcm = aiff.subarray(body + 8 + aiff.readUInt32BE(body), body + size);
    }
    at = body + size + (size % 2); // chunks are padded to an even length
  }
  if (!pcm || !channels || !rate || bits < 8 || bits > 32 || bits % 8) return null;

  const bytes = bits / 8;
  const data = Buffer.alloc(pcm.length - (pcm.length % bytes));
  for (let i = 0; i < data.length; i += bytes) {
    for (let b = 0; b < bytes; b++) data[i + b] = pcm[i + bytes - 1 - b];
  }
  // 8-bit AIFF is signed, 8-bit WAV unsigned.
  if (bytes === 1) for (let i = 0; i < data.length; i++) data[i] ^= 0x80;

  const header = Buffer.alloc(44);
  header.write('RIFF', 0, 'ascii');
  header.writeUInt32LE(36 + data.length, 4);
  header.write('WAVEfmt ', 8, 'ascii');
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20); // PCM
  header.writeUInt16LE(channels, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * channels * bytes, 28);
  header.writeUInt16LE(channels * bytes, 32);
  header.writeUInt16LE(bits, 34);
  header.write('data', 36, 'ascii');
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}
