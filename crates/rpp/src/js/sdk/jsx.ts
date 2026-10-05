// The JSX runtime for `.tsx` sources, imported as `#rpp/jsx` and `#rpp/jsx/jsx-runtime`.
// A tag is a component: a function called with its props whose result is the element.

/** Anything JSX accepts as a child. */
export type Child = JSX.Element | string | number | boolean | null | undefined | readonly Child[];

/** A function component. */
export type Component = (props: any) => unknown;

/** Call `type` with `props`. A `key`, passed separately when present, is an ordinary prop. */
export function jsx(
  type: Component | string,
  props: Record<string, unknown>,
  ...key: [key?: unknown]
): JSX.Element {
  if (typeof type !== "function") {
    throw new TypeError(`<${String(type)}>: JSX tags must be components`);
  }
  return type(key.length === 0 ? props : { key: key[0], ...props }) as JSX.Element;
}

export const jsxs: typeof jsx = jsx;

/** The classic factory, which `<Tag {...props} key="id" />` compiles to. */
export function createElement(
  type: Component | string,
  props: Record<string, unknown> | null,
  ...children: Child[]
): JSX.Element {
  const content =
    children.length === 0 ? {} : { children: children.length === 1 ? children[0] : children };
  return jsx(type, { ...props, ...content });
}

/** The children as a flat array, without `null`, `undefined` or booleans. */
export function Fragment(props: { children?: Child }): Child[] {
  const out: Child[] = [];
  const add = (child: Child): void => {
    if (Array.isArray(child)) {
      for (const item of child as readonly Child[]) {
        add(item);
      }
    } else if (child !== null && child !== undefined && typeof child !== "boolean") {
      out.push(child);
    }
  };
  add(props.children);
  return out;
}

export declare namespace JSX {
  /** Any component result. */
  interface Element {}
  type ElementType = Component;
  interface ElementChildrenAttribute {
    children: {};
  }
  interface IntrinsicElements {}
}
