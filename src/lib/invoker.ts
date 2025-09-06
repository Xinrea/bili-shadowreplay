import { invoke as tauri_invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { fetch as tauri_fetch } from "@tauri-apps/plugin-http";
import { convertFileSrc as tauri_convert } from "@tauri-apps/api/core";
import { listen as tauri_listen } from "@tauri-apps/api/event";
import { open as tauri_open } from "@tauri-apps/plugin-shell";
import { onOpenUrl as tauri_onOpenUrl } from "@tauri-apps/plugin-deep-link";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: any;
  }
}

const ENDPOINT = localStorage.getItem("endpoint") || "";
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
      window.open(`index_clip.html?id=${args.videoId}`, "_blank");
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
    return await getCurrentWebviewWindow().setTitle(title);
  }

  document.title = title;
}

async function convertFileSrc(filePath: string) {
  if (TAURI_ENV) {
    // 在客户端模式下，需要获取config来构建绝对路径
    try {
      const config = (await invoke("get_config")) as any;
      const absolutePath = `${config.output}/${filePath}`;
      return tauri_convert(absolutePath);
    } catch (error) {
      console.error("Failed to get config for file path conversion:", error);
      return tauri_convert(filePath);
    }
  }
  // 在headless模式下，保持完整的相对路径
  return `${ENDPOINT}/output/${filePath}`;
}

const coverCache: Map<string, string> = new Map();

async function get_cover(coverType: string, coverPath: string) {
  const config = (await invoke("get_config")) as any;
  if (coverType === "live") {
    const absolutePath = `${config.cache}/${coverPath}`;
    if (coverCache.has(absolutePath)) {
      return coverCache.get(absolutePath);
    }
    const url = tauri_convert(absolutePath);
    coverCache.set(absolutePath, url);
    return url;
  }

  if (coverType === "video") {
    const absolutePath = `${config.output}/${coverPath}`;
    if (coverCache.has(absolutePath)) {
      return coverCache.get(absolutePath);
    }
    const url = tauri_convert(absolutePath);
    coverCache.set(absolutePath, url);
    return url;
  }

  // exception
  throw new Error(`Invalid cover type: ${coverType}`);
}

let event_source: EventSource | null = null;
let reconnectTimeout: number | null = null;
const MAX_RECONNECT_ATTEMPTS = 5;
let reconnectAttempts = 0;

// 连接恢复回调列表
const connectionRestoreCallbacks: Array<() => void> = [];

function createEventSource() {
  if (TAURI_ENV) return;

  if (event_source) {
    event_source.close();
  }
  event_source = new EventSource(`${ENDPOINT}/api/sse`);

  event_source.onopen = () => {
    reconnectAttempts = 0;

    // 触发连接恢复回调
    connectionRestoreCallbacks.forEach((callback) => {
      try {
        callback();
      } catch (e) {
        console.error("[SSE] Connection restore callback error:", e);
      }
    });
  };

  event_source.onerror = (error) => {
    // 只有在连接真正关闭时才进行重连
    if (
      event_source.readyState === EventSource.CLOSED &&
      reconnectAttempts < MAX_RECONNECT_ATTEMPTS
    ) {
      reconnectAttempts++;
      const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 10000);

      reconnectTimeout = window.setTimeout(() => {
        createEventSource();
      }, delay);
    } else {
      console.error("[SSE] Max reconnection attempts reached, giving up");
    }
  };
}

// 注册连接恢复回调
function onConnectionRestore(callback: () => void) {
  connectionRestoreCallbacks.push(callback);
}

if (!TAURI_ENV) {
  createEventSource();
}

async function listen<T>(event: string, callback: (data: any) => void) {
  if (TAURI_ENV) {
    return await tauri_listen(event, callback);
  }

  event_source.addEventListener(event, (event_data) => {
    const data = JSON.parse(event_data.data);
    callback({
      type: event,
      payload: data,
    });
  });
}

async function open(url: string) {
  if (TAURI_ENV) {
    return await tauri_open(url);
  }
  window.open(url, "_blank");
}

async function close_window() {
  if (TAURI_ENV) {
    return await getCurrentWebviewWindow().close();
  }
  window.close();
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
  convertFileSrc,
  ENDPOINT,
  listen,
  open,
  log,
  close_window,
  onOpenUrl,
  onConnectionRestore,
  get_cover,
};
