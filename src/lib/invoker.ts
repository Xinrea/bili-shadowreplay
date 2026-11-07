import { invoke as tauri_invoke } from "@tauri-apps/api/core";
import { fetch as tauri_fetch } from "@tauri-apps/plugin-http";
import { open as tauri_open } from "@tauri-apps/plugin-shell";
import { onOpenUrl as tauri_onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { createEventClient } from "./events";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: any;
  }
}

function normalizeEndpoint(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

const ENDPOINT = normalizeEndpoint(localStorage.getItem("endpoint") || "");
const TAURI_ENV = typeof window.__TAURI_INTERNALS__ !== "undefined";

const log = {
  error: (...args: any[]) => {
    const message = args.map((arg) => JSON.stringify(arg)).join(" ");
    invoke("console_log", { level: "error", message });
    console.error(message);
  },
  warn: (...args: any[]) => {
    const message = args.map((arg) => JSON.stringify(arg)).join(" ");
    invoke("console_log", { level: "warn", message });
    console.warn(message);
  },
  info: (...args: any[]) => {
    const message = args.map((arg) => JSON.stringify(arg)).join(" ");
    invoke("console_log", { level: "info", message });
    console.info(message);
  },
  debug: (...args: any[]) => {
    const message = args.map((arg) => JSON.stringify(arg)).join(" ");
    invoke("console_log", { level: "debug", message });
    console.debug(message);
  },
};

async function invoke<T>(
  command: string,
  args?: Record<string, any>
): Promise<T> {
  try {
    if (TAURI_ENV) {
      // using tauri invoke
      return await tauri_invoke<T>(command, args);
    }

    if (command === "open_live") {
      console.log(args);
      // open new page to live_index.html
      window.open(
        `index_live.html?platform=${args.platform}&room_id=${args.roomId}&live_id=${args.liveId}`,
        "_blank"
      );
      return;
    }

    if (command === "open_clip") {
      window.open(`index_clip.html?material=${args.videoId}`, "_blank");
      return;
    }

    if (command === "open_project") {
      window.open(`index_clip.html?project=${args.projectId}`, "_blank");
      return;
    }

    let response = await fetch(`${ENDPOINT}/api/${command}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(args || {}),
    });

    // if status is 405, it means the command is not allowed
    if (response.status === 405) {
      throw new Error(
        `Command ${command} is not allowed, maybe bili-shadowreplay is running in readonly mode or HTTP method mismatch`
      );
    }
    if (!response.ok) {
      const error = await response.json().catch(() => ({
        message: `HTTP ${response.status}`,
      }));
      throw new Error(error.message || `HTTP error: ${response.status}`);
    }

    const resp = await response.json();
    if (resp.code !== 0) {
      throw new Error(resp.message);
    }

    return resp.data as T;
  } catch (error) {
    // 将 HTTP 错误转换为 Tauri 风格的错误
    throw new Error(`Failed to invoke ${command}:\n${error}`);
  }
}

async function get(url: string) {
  if (TAURI_ENV) {
    return await tauri_fetch(url);
  }
  const response = await fetch(`${ENDPOINT}/api/fetch`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      url,
      method: "GET",
      headers: {},
      body: null,
    }),
  });
  return response;
}

async function set_title(title: string) {
  if (TAURI_ENV) {
    // Not `setTitle()`: on macOS that resets the traffic light position.
    return await invoke("set_window_title", { title });
  }

  document.title = title;
}

let staticPort: Promise<number> | null = null;

async function static_origin() {
  if (TAURI_ENV) {
    // Share the first lookup across concurrent assets; failures remain retryable.
    staticPort ??= invoke<number>("get_static_port").catch((error) => {
      staticPort = null;
      throw error;
    });
    return `http://localhost:${await staticPort}`;
  }
  // In headless/web mode static assets are served by the same server as the
  // API, so follow the configured endpoint as-is. Rewriting the port here
  // breaks remapped ports, reverse proxies and HTTPS.
  return ENDPOINT;
}

async function get_static_url(base: string, path: string) {
  const encodedPath = path.split('/').map(encodeURIComponent).join('/');
  return `${await static_origin()}/${base}/${encodedPath}`;
}

/** Chunk `chunk` of the sound of `file`, relative to the output directory, for the project preview. */
async function get_preview_audio_url(file: string, chunk: number) {
  return `${await static_origin()}/preview_audio?file=${encodeURIComponent(file)}&chunk=${chunk}`;
}

const { listen } = createEventClient({ tauri: TAURI_ENV, endpoint: ENDPOINT });

async function open(url: string) {
  if (TAURI_ENV) {
    return await tauri_open(url);
  }
  window.open(url, "_blank");
}

async function onOpenUrl(func: (urls: string[]) => void) {
  if (TAURI_ENV) {
    return await tauri_onOpenUrl(func);
  }
}

export {
  invoke,
  get,
  set_title,
  TAURI_ENV,
  ENDPOINT,
  listen,
  open,
  log,
  onOpenUrl,
  get_static_url,
  get_preview_audio_url,
  normalizeEndpoint,
};
