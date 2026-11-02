import { getContext, setContext } from "svelte";

const key = Symbol("page-visibility");

/** Pages stay mounted while hidden; descendants must suspend transient UI. */
export function providePageVisibility(visible: () => boolean) {
  setContext(key, visible);
}

export function pageVisibility(): () => boolean {
  return getContext<() => boolean>(key) ?? (() => true);
}
