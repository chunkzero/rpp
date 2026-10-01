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
      throw error?.name === "RppInvalidResult" ? error : invalid(`Result is not JSON: ${error?.message}`);
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
  const dateString = Function.prototype.call.bind(NativeDate.prototype.toString);
  const controlledDate = new Proxy(NativeDate, {
    apply: () => dateString(new NativeDate(now())),
    construct: (target, args, newTarget) => construct(target, args.length ? args : [now()], newTarget),
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
      if (args[1]?.maxByteLength !== undefined) throw new Error("Resizable buffers are unavailable");
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
  for (const prototype of [String.prototype, Number.prototype, BigInt.prototype, Array.prototype]) {
    for (const name of ["localeCompare", "toLocaleString", "toLocaleLowerCase", "toLocaleUpperCase"]) {
      if (name in prototype)
        Object.defineProperty(prototype, name, { value: unavailable, writable: false, configurable: false });
    }
  }
  Object.defineProperty(globalThis, "__rpp", {
    value: freeze({
      call(name, value, bytes) {
        if (bytes !== undefined && !ArrayBuffer.isView(bytes)) throw new TypeError("Expected a Uint8Array");
        const json = hostCall(String(name), stringify(value === undefined ? null : value), bytes !== undefined, bytes ?? noBytes);
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
