// A store-only zip writer: no compression (photos do not compress), one file after another, so the whole
// archive never has to be held at once when the browser lets us write to a file as we go.

const TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(bytes) {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function dosTime(date) {
  const time = (date.getHours() << 11) | (date.getMinutes() << 5) | (date.getSeconds() >> 1);
  const day = ((date.getFullYear() - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate();
  return [time & 0xffff, day & 0xffff];
}

export const ZIP_FILE_LIMIT = 0xffffffff;

// `sink(bytes)` is called with each piece in order. Returns an object to add files and finish.
export function zipWriter(sink) {
  const central = [];
  let offset = 0;
  const encoder = new TextEncoder();
  const emit = async (bytes) => {
    await sink(bytes);
    offset += bytes.length;
  };
  return {
    async add(name, data, when = new Date()) {
      if (data.length > ZIP_FILE_LIMIT) throw new Error('a file is too large for a zip');
      const nameBytes = encoder.encode(name);
      const [time, day] = dosTime(when);
      const crc = crc32(data);
      const head = new DataView(new ArrayBuffer(30));
      head.setUint32(0, 0x04034b50, true);
      head.setUint16(4, 20, true);
      head.setUint16(6, 0x0800, true); // names are UTF-8
      head.setUint16(8, 0, true); // stored
      head.setUint16(10, time, true);
      head.setUint16(12, day, true);
      head.setUint32(14, crc, true);
      head.setUint32(18, data.length, true);
      head.setUint32(22, data.length, true);
      head.setUint16(26, nameBytes.length, true);
      head.setUint16(28, 0, true);
      central.push({ nameBytes, crc, size: data.length, time, day, offset });
      await emit(new Uint8Array(head.buffer));
      await emit(nameBytes);
      await emit(data);
    },
    async finish() {
      const start = offset;
      for (const e of central) {
        const d = new DataView(new ArrayBuffer(46));
        d.setUint32(0, 0x02014b50, true);
        d.setUint16(4, 20, true);
        d.setUint16(6, 20, true);
        d.setUint16(8, 0x0800, true);
        d.setUint16(10, 0, true);
        d.setUint16(12, e.time, true);
        d.setUint16(14, e.day, true);
        d.setUint32(16, e.crc, true);
        d.setUint32(20, e.size, true);
        d.setUint32(24, e.size, true);
        d.setUint16(28, e.nameBytes.length, true);
        d.setUint32(42, e.offset, true);
        await emit(new Uint8Array(d.buffer));
        await emit(e.nameBytes);
      }
      const size = offset - start;
      const end = new DataView(new ArrayBuffer(22));
      end.setUint32(0, 0x06054b50, true);
      end.setUint16(8, central.length, true);
      end.setUint16(10, central.length, true);
      end.setUint32(12, size, true);
      end.setUint32(16, start, true);
      await emit(new Uint8Array(end.buffer));
    },
  };
}
