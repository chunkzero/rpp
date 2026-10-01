(() => {
  "use strict";
  const core = Deno.core;
  const encoding = core.loadExtScript("ext:deno_web/08_text_encoding.js");
  const url = core.loadExtScript("ext:deno_web/00_url.js");
  const base64 = core.loadExtScript("ext:deno_web/05_base64.js");
  const random = core.ops.op_rpp_random;
  const log = core.ops.op_rpp_log;
  const Uint8 = Uint8Array;
  const encoder = new encoding.TextEncoder();
  const encodeInto = encoder.encodeInto.bind(encoder);
  class TextEncoder {
    get encoding() {
      return "utf-8";
    }
    encode(input = "") {
      input = String(input);
      // Allocate through the isolate allocator; core.encode adopts an untracked Vec.
      const bytes = new Uint8(input.length * 3);
      const { written } = encodeInto(input, bytes);
      return bytes.subarray(0, written);
    }
    encodeInto(source, destination) {
      return encodeInto(source, destination);
    }
  }
  Object.assign(globalThis, {
    TextEncoder,
    TextDecoder: encoding.TextDecoder,
    URL: url.URL,
    URLSearchParams: url.URLSearchParams,
    atob: base64.atob,
    btoa: base64.btoa,
  });
  delete url.URL.createObjectURL;
  delete url.URL.revokeObjectURL;
  const levels = ["debug", "log", "info", "warn", "error"];
  globalThis.console = Object.freeze(
    Object.fromEntries(
      levels.map((level) => [
        level,
        (...values) => {
          const line = values
            .map((value) => {
              if (typeof value === "string") return value;
              try {
                return JSON.stringify(value);
              } catch {
                return String(value);
              }
            })
            .join(" ");
          log(level, line);
        },
      ]),
    ),
  );
  const integerArrays = new Set([
    Int8Array,
    Uint8Array,
    Uint8ClampedArray,
    Int16Array,
    Uint16Array,
    Int32Array,
    Uint32Array,
    BigInt64Array,
    BigUint64Array,
  ]);
  const getRandomValues = (array) => {
    if (!ArrayBuffer.isView(array) || !integerArrays.has(Object.getPrototypeOf(array).constructor))
      throw new TypeError("Expected an integer typed array");
    if (array.byteLength > 65536) throw new RangeError("Random byte limit exceeded");
    const bytes = new Uint8(array.buffer, array.byteOffset, array.byteLength);
    for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(random() * 256);
    if (bytes.length === 0) random();
    return array;
  };
  globalThis.crypto = Object.freeze({
    getRandomValues,
    randomUUID() {
      const bytes = getRandomValues(new Uint8(16));
      bytes[6] = (bytes[6] & 15) | 64;
      bytes[8] = (bytes[8] & 63) | 128;
      const hex = Array.from(bytes, (n) => n.toString(16).padStart(2, "0")).join("");
      return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    },
  });
})();
