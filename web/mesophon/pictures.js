// Pictures in a note (T-629): a photo or screenshot picked on the phone
// becomes the PNG the desk keeps, named `[Image #N]` in the draft as the
// desk's editor names it, and linked to the host's id only once it is sent.
// The page draws its own thumbnails on a canvas: the relay's CSP admits no
// `blob:` or `data:` image, and a decoded bitmap needs neither.

// The host's limits (`mesimon_core::attachment`) and one `upload` piece
// (`mesophon::PICTURE_CHUNK_BYTES`), cut so a sealed piece fits a frame.
export const PICTURE_MAX_BYTES = 10 * 1024 * 1024;
export const PICTURE_MAX_PIXELS = 25_000_000;
export const PICTURE_CHUNK_BYTES = 80 * 1024;
// A phone photo is 12 megapixels; as a PNG at that size it outgrows the
// host's 10 MiB. The long edge is cut to this first, which keeps a phone
// screenshot sharp enough to read.
const LONG_EDGE = 2048;
const THUMB = 160;

const TOKEN = /\[Image #(\d+)\](?!\()/g;

// The numbers a draft names and has not linked yet, in order of first use.
export function unlinked(text) {
  const seen = [];
  for (const [, n] of text.matchAll(TOKEN)) if (!seen.includes(Number(n))) seen.push(Number(n));
  return seen;
}

// The number the next picture takes: past every one the text names,
// linked or not, and every one the draft still holds.
export function nextNumber(text, held = []) {
  let top = 0;
  for (const [, n] of text.matchAll(/\[Image #(\d+)\]/g)) top = Math.max(top, Number(n));
  for (const n of held) top = Math.max(top, n);
  return top + 1;
}

// The draft's words with each sent picture linked to its host id.
export function linked(text, ids) {
  return text.replace(TOKEN, (whole, n) => (ids.has(Number(n)) ? `${whole}(mesimon-attachment:${ids.get(Number(n))})` : whole));
}

// The draft's words without one picture's names.
export function withoutPicture(text, n) {
  return text.replace(new RegExp(`[ \\t]?\\[Image #${n}\\](?!\\()`, "g"), "");
}

// `[Image #N]` placed at `at`, with a space either side where words touch.
export function insertToken(text, at, n) {
  const where = Math.max(0, Math.min(at ?? text.length, text.length));
  const before = text.slice(0, where);
  const after = text.slice(where);
  const token = `${before && !/\s$/.test(before) ? " " : ""}[Image #${n}]${after && !/^\s/.test(after) ? " " : ""}`;
  return { text: before + token + after, at: where + token.length };
}

function canvas(width, height) {
  if (typeof OffscreenCanvas === "function") return new OffscreenCanvas(width, height);
  const c = document.createElement("canvas");
  c.width = width;
  c.height = height;
  return c;
}

async function pngOf(source, width, height) {
  const c = canvas(width, height);
  c.getContext("2d").drawImage(source, 0, 0, width, height);
  const blob = c.convertToBlob
    ? await c.convertToBlob({ type: "image/png" })
    : await new Promise((resolve) => c.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("could not make a PNG");
  return new Uint8Array(await blob.arrayBuffer());
}

// A picked file as the host's PNG, its size, and a small bitmap to show.
// Anything the browser can decode goes in; what comes out is a PNG within
// the host's limits, upright, and without the photo's metadata.
export async function picture(file) {
  let bitmap;
  try {
    bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
  } catch {
    throw new Error("This browser cannot read that picture.");
  }
  try {
    let scale = Math.min(1, LONG_EDGE / Math.max(bitmap.width, bitmap.height));
    scale = Math.min(scale, Math.sqrt(PICTURE_MAX_PIXELS / (bitmap.width * bitmap.height)));
    for (;;) {
      const width = Math.max(1, Math.round(bitmap.width * scale));
      const height = Math.max(1, Math.round(bitmap.height * scale));
      const bytes = await pngOf(bitmap, width, height);
      if (bytes.length <= PICTURE_MAX_BYTES) {
        const t = Math.min(1, THUMB / Math.max(width, height));
        const thumb = await createImageBitmap(bitmap, {
          resizeWidth: Math.max(1, Math.round(bitmap.width * scale * t)),
          resizeHeight: Math.max(1, Math.round(bitmap.height * scale * t)),
          resizeQuality: "medium",
        });
        return { bytes, width, height, thumb };
      }
      scale *= 0.75;
    }
  } finally {
    bitmap.close?.();
  }
}

// One piece of `bytes` as the `upload` op's base64.
export function pieceOf(bytes, offset) {
  const slice = bytes.subarray(offset, offset + PICTURE_CHUNK_BYTES);
  let binary = "";
  for (let i = 0; i < slice.length; i += 0x8000) binary += String.fromCharCode(...slice.subarray(i, i + 0x8000));
  return { data: btoa(binary), end: offset + slice.length };
}
