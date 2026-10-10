// Renders the GetKeys key mark (dark tile, white key, orange bit) to PNG without image dependencies.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { crc32, deflateSync } from 'node:zlib';

const TILE = [24, 24, 27];
const KEY = [250, 250, 250];
const SIGNAL = [234, 88, 12];

function roundedRect(x, y, x0, y0, x1, y1, r) {
  const cx = Math.min(Math.max(x, x0 + r), x1 - r);
  const cy = Math.min(Math.max(y, y0 + r), y1 - r);
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r && x >= x0 && x <= x1 && y >= y0 && y <= y1;
}

function capsule(x, y, ax, bx, cy, r) {
  const px = Math.min(Math.max(x, ax), bx);
  return (x - px) ** 2 + (y - cy) ** 2 <= r * r;
}

function sample(x, y) {
  if (!roundedRect(x, y, 0.02, 0.02, 0.98, 0.98, 0.22)) return null;
  const d = Math.hypot(x - 0.35, y - 0.5);
  if (d <= 0.06) return SIGNAL;
  const ring = d >= 0.095 && d <= 0.175;
  const shaft = capsule(x, y, 0.5, 0.78, 0.5, 0.045);
  const tooth = roundedRect(x, y, 0.665, 0.47, 0.755, 0.665, 0.03);
  return ring || shaft || tooth ? KEY : TILE;
}

export function renderKeyIcon(size) {
  const n = 6;
  const rows = [];
  for (let py = 0; py < size; py++) {
    const row = Buffer.alloc(1 + size * 4);
    for (let px = 0; px < size; px++) {
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      for (let sy = 0; sy < n; sy++) {
        for (let sx = 0; sx < n; sx++) {
          const color = sample((px + (sx + 0.5) / n) / size, (py + (sy + 0.5) / n) / size);
          if (!color) continue;
          r += color[0];
          g += color[1];
          b += color[2];
          a += 1;
        }
      }
      const o = 1 + px * 4;
      if (a > 0) {
        row[o] = Math.round(r / a);
        row[o + 1] = Math.round(g / a);
        row[o + 2] = Math.round(b / a);
        row[o + 3] = Math.round((a / (n * n)) * 255);
      }
    }
    rows.push(row);
  }
  return encodePng(size, size, Buffer.concat(rows));
}

function chunk(type, data) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, 'ascii');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])), 0);
  return Buffer.concat([head, data, crc]);
}

function encodePng(width, height, raw) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** Writes `icon-<size>.png` for each size into `dir`. */
export function writeIcons(dir, sizes) {
  mkdirSync(dir, { recursive: true });
  for (const size of sizes) writeFileSync(join(dir, `icon-${size}.png`), renderKeyIcon(size));
}
