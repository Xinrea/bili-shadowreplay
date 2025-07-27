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

    const response = await fetch(`${ENDPOINT}/api/${command}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(args || {}),
    });
    // if status is 405, it means the command is not allowed
    if (response.status === 405) {
      throw new Error(
        `Command ${command} is not allowed, maybe bili-shadowreplay is running in readonly mode`
      );
    }
    if (!response.ok) {
      const error = await response.json();
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

function convertFileSrc(filePath: string) {
  if (TAURI_ENV) {
    return tauri_convert(filePath);
  }
  return `${ENDPOINT}/output/${filePath.split("/").pop()}`;
}

let event_source: EventSource | null = null;

if (!TAURI_ENV) {
  event_source = new EventSource(`${ENDPOINT}/api/sse`);

  event_source.onopen = () => {
    console.log("EventSource connection opened");
  };

  event_source.onerror = (error) => {
    console.error("EventSource error:", error);
  };
}

async function listen<T>(event: string, callback: (data: any) => void) {
  if (TAURI_ENV) {
    return await tauri_listen(event, callback);
  }

  event_source.addEventListener(event, (event_data) => {
    const data = JSON.parse(event_data.data);
    console.log("Parsed EventSource data:", data);
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
};
