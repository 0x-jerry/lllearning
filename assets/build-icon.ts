// Rasterizes assets/icon.svg into assets/icon.png and assets/icon.ico.
//
//   bun run build:icon
//
// resvg turns the SVG into pixels (Bun.Image cannot decode SVG), then Bun.Image
// does the downscaling and PNG encoding.

import { Resvg } from "@resvg/resvg-js";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const dir = join(import.meta.dir, "..", "assets");
const svg = readFileSync(join(dir, "icon.svg"));

// Render once at the largest size and let Bun downscale from there.
const base = new Resvg(svg, { fitTo: { mode: "width", value: 256 } })
  .render()
  .asPng();

const png = async (size: number): Promise<Uint8Array> =>
  size === 256 ? base : new Bun.Image(base).resize(size, size).png().bytes();

/** Packs PNGs into a PNG-compressed .ico (Vista and later). */
function buildIco(images: { size: number; png: Uint8Array }[]): Uint8Array {
  const header = Buffer.alloc(6 + 16 * images.length);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(images.length, 4);

  let offset = header.length;
  images.forEach(({ size, png }, i) => {
    const entry = 6 + 16 * i;
    header.writeUInt8(size >= 256 ? 0 : size, entry); // width, 0 means 256
    header.writeUInt8(size >= 256 ? 0 : size, entry + 1); // height
    header.writeUInt8(0, entry + 2); // palette size
    header.writeUInt8(0, entry + 3); // reserved
    header.writeUInt16LE(1, entry + 4); // color planes
    header.writeUInt16LE(32, entry + 6); // bits per pixel
    header.writeUInt32LE(png.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += png.length;
  });

  return Buffer.concat([header, ...images.map((image) => image.png)]);
}

const sizes = [16, 24, 32, 48, 64, 128, 256];
const images = await Promise.all(sizes.map(async (size) => ({ size, png: await png(size) })));

const iconPng = images.at(-1)!.png;
const iconIco = buildIco(images);
writeFileSync(join(dir, "icon.png"), iconPng);
writeFileSync(join(dir, "icon.ico"), iconIco);

console.log(`icon.png  256x256, ${iconPng.length} bytes`);
console.log(`icon.ico  ${sizes.join("/")}, ${iconIco.length} bytes`);
