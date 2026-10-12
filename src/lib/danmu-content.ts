import type { DanmuEntry } from "./interface";

// Register image CDNs explicitly when adding providers. Recorded metadata
// must not make the user's browser send requests to arbitrary/local services.
const EMOJI_CDN_DOMAINS = ["hdslb.com", "biliimg.com"];

/** Only load images over HTTPS from supported platforms' trusted CDNs. */
export function emojiPresentation(entry: DanmuEntry) {
  if (entry.type !== "danmu" || !entry.emoji_content?.url?.trim()) return null;
  const emoji = entry.emoji_content;
  let url: URL;
  try {
    url = new URL(emoji.url.trim().replace(/^\/\//, "https://"));
  } catch {
    return null;
  }
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username || url.password || url.port ||
    !EMOJI_CDN_DOMAINS.some((domain) =>
      url.hostname === domain || url.hostname.endsWith(`.${domain}`),
    )
  ) {
    return null;
  }
  url.protocol = "https:";

  const ratio =
    Number.isFinite(emoji.width) && Number.isFinite(emoji.height) &&
    emoji.width! > 0 && emoji.height! > 0
      ? emoji.width! / emoji.height!
      : 1;
  return {
    url: url.href,
    // Reserve the footprint before loading, keeping scrolling measurements
    // and the virtual list's fixed row heights stable when the image appears.
    widthEm: 1.5 * Math.min(3, Math.max(0.25, ratio)),
    heightEm: 1.5,
  };
}

/** Shared by the floating overlay and the Svelte list action below. */
export function renderDanmuContent(node: HTMLElement, entry: DanmuEntry) {
  const presentation = emojiPresentation(entry);
  node.textContent = entry.content;
  if (!presentation) return () => {};

  const box = document.createElement("span");
  box.style.display = "inline-grid";
  box.style.gridTemplateColumns = "minmax(0, 1fr)";
  box.style.gridTemplateRows = "minmax(0, 1fr)";
  box.style.verticalAlign = "middle";
  box.style.width = `${presentation.widthEm}em`;
  box.style.height = `${presentation.heightEm}em`;
  box.style.lineHeight = `${presentation.heightEm}em`;
  box.title = entry.content;

  const fallback = document.createElement("span");
  fallback.textContent = entry.content;
  fallback.style.gridArea = "1 / 1";
  fallback.style.overflow = "hidden";
  fallback.style.textOverflow = "ellipsis";
  fallback.style.whiteSpace = "nowrap";

  const image = document.createElement("img");
  image.alt = entry.content;
  image.referrerPolicy = "no-referrer";
  image.draggable = false;
  image.style.gridArea = "1 / 1";
  image.style.width = "100%";
  image.style.height = "100%";
  image.style.objectFit = "contain";
  image.style.visibility = "hidden";
  const onLoad = () => {
    image.style.visibility = "visible";
    fallback.style.visibility = "hidden";
  };
  const onError = () => {
    // Use ordinary text sizing on failure, not an unreadable label squeezed
    // into the image's small footprint.
    node.textContent = entry.content;
    dispose();
  };
  const dispose = () => {
    image.removeEventListener("load", onLoad);
    image.removeEventListener("error", onError);
  };
  image.addEventListener("load", onLoad);
  image.addEventListener("error", onError);
  box.append(fallback, image);
  node.replaceChildren(box);
  image.src = presentation.url;

  return dispose;
}

/** Update and dispose image listeners when virtualized rows are reused. */
export function danmuContent(node: HTMLElement, entry: DanmuEntry) {
  let dispose = renderDanmuContent(node, entry);
  return {
    update(next: DanmuEntry) {
      dispose();
      dispose = renderDanmuContent(node, next);
    },
    destroy() {
      dispose();
    },
  };
}
