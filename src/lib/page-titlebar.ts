import { getContext, setContext, type Snippet } from "svelte";

/** What a main-window page shows in the window's title bar. */
export interface PageTitlebarContent {
  title: string;
  /** Shown after the title, e.g. counts. */
  subtitle?: Snippet;
  /** Page actions at the right of the title bar. */
  actions?: Snippet;
}

type Sink = (content: PageTitlebarContent | null) => void;

const key = Symbol("page-titlebar");

/** Called by `PageView`, which scopes one page. */
export function providePageTitlebar(sink: Sink) {
  setContext(key, sink);
}

/** Called by `PageTitlebar` inside a page. */
export function pageTitlebarSink(): Sink | undefined {
  return getContext<Sink | undefined>(key);
}
