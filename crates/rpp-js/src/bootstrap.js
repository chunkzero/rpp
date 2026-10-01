(() => {
  "use strict";
  const ops = Deno.core.ops;
  const now = ops.op_rpp_now;
  const random = ops.op_rpp_random;
  const hostCall = ops.op_rpp_host;
  const pendingLength = ops.op_rpp_pending_length;
  const takePending = ops.op_rpp_take_pending;
  const Uint8 = Uint8Array;
  const freeze = Object.freeze;
  const parse = JSON.parse;
  const stringify = JSON.stringify;
  const finite = Number.isFinite;
  const noBytes = new Uint8(0);

  const invalid = (message) => {
    const error = new Error(message);
    error.name = "RppInvalidResult";
    return error;
  };
  const serialize = (value) => {
    try {
      return stringify(value, (_, item) => {
        const type = typeof item;
        if (
          type === "function" ||
          type === "symbol" ||
          type === "bigint" ||
          (type === "number" && !finite(item))
        )
          throw invalid(`Result is not JSON: ${type} values are not allowed`);
        return item;
      });
    } catch (error) {
      throw error?.name === "RppInvalidResult"
        ? error
        : invalid(`Result is not JSON: ${error?.message}`);
    }
  };
  const readPending = () => {
    const length = pendingLength();
    if (length < 0) return undefined;
    const bytes = new Uint8(length);
    takePending(bytes);
    return bytes;
  };

  const construct = Reflect.construct;
  const NativeDate = Date;
  const proto = NativeDate.prototype;
  const lock = (target, name, value) =>
    Object.defineProperty(target, name, { value, writable: false, configurable: false });
  const utcString = Function.prototype.call.bind(proto.toUTCString);
  const utcParts = (date) => {
    const text = utcString(date);
    return text === "Invalid Date" ? undefined : text.split(" ");
  };
  const datePart = ([weekday, day, month, year]) =>
    `${weekday.slice(0, 3)} ${month} ${day} ${year}`;
  const timePart = (parts) => `${parts[4]} GMT+0000 (UTC)`;
  const dateString = (date) => {
    const parts = utcParts(date);
    return parts ? `${datePart(parts)} ${timePart(parts)}` : "Invalid Date";
  };
  lock(proto, "toString", function toString() {
    return dateString(this);
  });
  lock(proto, "toDateString", function toDateString() {
    const parts = utcParts(this);
    return parts ? datePart(parts) : "Invalid Date";
  });
  lock(proto, "toTimeString", function toTimeString() {
    const parts = utcParts(this);
    return parts ? timePart(parts) : "Invalid Date";
  });
  for (const unit of ["FullYear", "Month", "Date", "Hours", "Minutes", "Seconds", "Milliseconds"]) {
    lock(proto, `get${unit}`, proto[`getUTC${unit}`]);
    lock(proto, `set${unit}`, proto[`setUTC${unit}`]);
  }
  lock(proto, "getDay", proto.getUTCDay);
  lock(proto, "getYear", function getYear() {
    return this.getUTCFullYear() - 1900;
  });
  lock(proto, "setYear", function setYear(year) {
    const value = Number(year);
    return this.setUTCFullYear(value >= 0 && value <= 99 ? value + 1900 : value);
  });
  lock(proto, "getTimezoneOffset", function getTimezoneOffset() {
    return this.getTime() - this.getTime();
  });
  // Only ISO formats parse, and those without an offset are UTC, so results never
  // depend on the host timezone.
  const isoDate = /^(?:[+-]\d{6}|\d{4})(?:-\d\d(?:-\d\d)?)?$/;
  const isoDateTime =
    /^(?:[+-]\d{6}|\d{4})-\d\d-\d\dT\d\d:\d\d(?::\d\d(?:\.\d+)?)?(Z|[+-]\d\d:\d\d)?$/;
  const nativeParse = NativeDate.parse;
  const parseUtc = (value) => {
    const text = String(value);
    if (isoDate.test(text)) return nativeParse(text);
    const match = isoDateTime.exec(text);
    return match ? nativeParse(match[1] ? text : `${text}Z`) : NaN;
  };
  const toPrimitive = (value) => {
    if ((typeof value !== "object" || value === null) && typeof value !== "function") return value;
    const exotic = value[Symbol.toPrimitive];
    if (exotic !== undefined && exotic !== null) return exotic.call(value, "default");
    for (const name of ["valueOf", "toString"]) {
      const method = value[name];
      if (typeof method === "function") {
        const result = method.call(value);
        if ((typeof result !== "object" || result === null) && typeof result !== "function")
          return result;
      }
    }
    throw new TypeError("Cannot convert object to primitive value");
  };
  const timeOf = (value) => {
    if (value instanceof NativeDate) return value.getTime();
    const primitive = toPrimitive(value);
    return typeof primitive === "string" ? parseUtc(primitive) : primitive;
  };
  lock(NativeDate, "parse", parseUtc);
  const controlledDate = new Proxy(NativeDate, {
    apply: () => dateString(new NativeDate(now())),
    construct: (target, args, newTarget) => {
      const input =
        args.length >= 2
          ? [NativeDate.UTC(...args)]
          : args.length === 1
            ? [timeOf(args[0])]
            : [now()];
      return construct(target, input, newTarget);
    },
  });
  Object.defineProperty(NativeDate, "now", { value: now, writable: false, configurable: false });
  Object.defineProperty(NativeDate.prototype, "constructor", {
    value: controlledDate,
    writable: false,
    configurable: false,
  });
  globalThis.Date = controlledDate;
  Object.defineProperty(Math, "random", { value: random, writable: false, configurable: false });
  const boundedBuffer = new Proxy(ArrayBuffer, {
    construct: (target, args, newTarget) => {
      if (args[1]?.maxByteLength !== undefined)
        throw new Error("Resizable buffers are unavailable");
      return construct(target, args.length ? [args[0]] : [], newTarget);
    },
  });
  Object.defineProperty(ArrayBuffer.prototype, "constructor", {
    value: boundedBuffer,
    writable: false,
    configurable: false,
  });
  globalThis.ArrayBuffer = boundedBuffer;
  const unavailable = () => {
    throw new Error("Locale-sensitive APIs are unavailable");
  };
  for (const prototype of [
    String.prototype,
    Number.prototype,
    BigInt.prototype,
    Array.prototype,
    Date.prototype,
  ]) {
    for (const name of [
      "localeCompare",
      "toLocaleString",
      "toLocaleLowerCase",
      "toLocaleUpperCase",
      "toLocaleDateString",
      "toLocaleTimeString",
    ]) {
      if (name in prototype)
        Object.defineProperty(prototype, name, {
          value: unavailable,
          writable: false,
          configurable: false,
        });
    }
  }
  Object.defineProperty(globalThis, "__rpp", {
    value: freeze({
      call(name, value, bytes) {
        if (bytes !== undefined && !ArrayBuffer.isView(bytes))
          throw new TypeError("Expected a Uint8Array");
        const json = hostCall(
          String(name),
          stringify(value === undefined ? null : value),
          bytes !== undefined,
          bytes ?? noBytes,
        );
        return { value: parse(json), bytes: readPending() };
      },
    }),
    enumerable: false,
    writable: false,
    configurable: false,
  });
  for (const name of [
    "Deno",
    "__bootstrap",
    "__infra",
    "Temporal",
    "Intl",
    "performance",
    "WeakRef",
    "FinalizationRegistry",
    "WebAssembly",
    "SharedArrayBuffer",
  ]) {
    delete globalThis[name];
  }
  return async (handler, argsJson) => {
    const bytes = readPending();
    const result = await handler(parse(argsJson), bytes);
    if (result === undefined || result instanceof Uint8) return result;
    return serialize(result);
  };
})();
