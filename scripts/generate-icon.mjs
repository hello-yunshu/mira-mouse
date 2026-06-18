// SPDX-License-Identifier: AGPL-3.0-or-later
// Generate a minimal 1024x1024 placeholder PNG for Tauri icon generation.
import { createWriteStream, mkdirSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

const SIZE = 1024;
const rgba = [0x4a, 0x46, 0x54, 0xff]; // muted purple-grey

function chunk(type, data) {
  const len = data.length;
  const buf = Buffer.alloc(12 + len);
  buf.writeUInt32BE(len, 0);
  buf.write(type, 4, 4, 'ascii');
  data.copy(buf, 8);
  const crc = Buffer.alloc(4);
  // CRC-32 computed over type + data
  const typeData = buf.subarray(4, 8 + len);
  crc.writeUInt32BE(crc32(typeData), 0);
  crc.copy(buf, 8 + len);
  return buf;
}

const CRC_TABLE = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let c = i;
  for (let k = 0; k < 8; k++) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  CRC_TABLE[i] = c;
}

function crc32(buf) {
  let c = ~0;
  for (let i = 0; i < buf.length; i++) {
    c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  }
  return ~c >>> 0;
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0); // width
ihdr.writeUInt32BE(SIZE, 4); // height
ihdr.writeUInt8(8, 8); // bit depth
ihdr.writeUInt8(6, 9); // color type: RGBA
ihdr.writeUInt8(0, 10); // compression
ihdr.writeUInt8(0, 11); // filter
ihdr.writeUInt8(0, 12); // interlace

const row = Buffer.concat([Buffer.from([0]), Buffer.alloc(SIZE * 4).fill(Buffer.from(rgba))]);
const raw = Buffer.alloc(row.length * SIZE);
for (let y = 0; y < SIZE; y++) {
  row.copy(raw, y * row.length);
}
const idat = deflateSync(raw, { level: 9 });

const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const iconDir = new URL('../src-tauri/icons', import.meta.url).pathname;
mkdirSync(iconDir, { recursive: true });
const out = createWriteStream(new URL('../src-tauri/icons/icon.png', import.meta.url));
out.write(signature);
out.write(chunk('IHDR', ihdr));
out.write(chunk('IDAT', idat));
out.write(chunk('IEND', Buffer.alloc(0)));
out.end();
console.log('generated src-tauri/icons/icon.png');
