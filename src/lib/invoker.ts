import { invoke as tauri_invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { fetch as tauri_fetch } from "@tauri-apps/plugin-http";
import { convertFileSrc as tauri_convert } from "@tauri-apps/api/core";
import { listen as tauri_listen } from "@tauri-apps/api/event";
import { open as tauri_open } from "@tauri-apps/plugin-shell";
import { onOpenUrl as tauri_onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { io, Socket } from "socket.io-client";

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
  if (TAURI_ENV) {
    if (coverType === "cache") {
      const absolutePath = `${config.cache}/${coverPath}`;
      if (coverCache.has(absolutePath)) {
        return coverCache.get(absolutePath);
      }
      const url = tauri_convert(absolutePath);
      coverCache.set(absolutePath, url);
      return url;
    }

    if (coverType === "output") {
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

  return `${ENDPOINT}/${coverType}/${coverPath}`;
}

let socket: Socket | null = null;

// Socket.IO 事件监听器映射
const eventListeners: Map<string, Array<(data: any) => void>> = new Map();

function createSocket() {
  if (socket) {
    socket.disconnect();
  }

  // 构建 Socket.IO URL
  console.log("endpoint:", ENDPOINT);
  const socketUrl = ENDPOINT;
  socket = io(`${socketUrl}/ws`, {
    transports: ["websocket", "polling"],
    autoConnect: true,
    reconnection: true,
  });

  socket.on("connect", () => {
    console.log("[Socket.IO] Connected to server");
  });

  socket.on("disconnect", (reason) => {
    console.log("[Socket.IO] Disconnected from server:", reason);
  });

  socket.on("connect_error", (error) => {
    console.error("[Socket.IO] Connection error:", error);
  });

  // 监听服务器发送的事件
  socket.on("progress", (data) => {
    const eventType = data.event || "message";

    // 触发对应的事件监听器
    const listeners = eventListeners.get(eventType);
    if (listeners) {
      listeners.forEach((callback) => {
        try {
          callback({
            type: eventType,
            payload: data.data,
          });
        } catch (e) {
          console.error(
            `[Socket.IO] Event listener error for ${eventType}:`,
            e
          );
        }
      });
    }
  });

  socket.on("danmu", (data) => {
    // 触发对应的事件监听器
    const listeners = eventListeners.get("danmu");
    if (listeners) {
      listeners.forEach((callback) => {
        try {
          callback({
            type: "danmu",
            payload: data.data,
          });
        } catch (e) {
          console.error(`[Socket.IO] Event listener error for danmu:`, e);
        }
      });
    }
  });
}

if (!TAURI_ENV) {
  createSocket();
}

async function listen<T>(event: string, callback: (data: any) => void) {
  if (TAURI_ENV) {
    return await tauri_listen(event, callback);
  }

  // 将事件监听器添加到映射中
  if (!eventListeners.has(event)) {
    eventListeners.set(event, []);
  }
  eventListeners.get(event)!.push(callback);

  // 返回一个清理函数
  return () => {
    const listeners = eventListeners.get(event);
    if (listeners) {
      const index = listeners.indexOf(callback);
      if (index > -1) {
        listeners.splice(index, 1);
      }
      // 如果没有监听器了，删除这个事件
      if (listeners.length === 0) {
        eventListeners.delete(event);
      }
    }
  };
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
  get_cover,
};
