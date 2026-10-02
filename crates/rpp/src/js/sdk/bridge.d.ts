// The host bridge rpp injects into plugin runtimes.

declare const __rpp: {
  call(
    name: string,
    value: unknown,
    bytes?: Uint8Array,
  ): { value: any; bytes: Uint8Array | undefined };
};
