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

    // 需要使用 GET 方法的命令列表
    const GET_COMMANDS = ['get_import_progress'];
    
    let response: Response;
    
    if (GET_COMMANDS.includes(command)) {
      // 使用 GET 方法
      const queryParams = args && Object.keys(args).length > 0 
        ? `?${new URLSearchParams(args).toString()}` 
        : '';
      response = await fetch(`${ENDPOINT}/api/${command}${queryParams}`, {
        method: "GET",
        headers: {
          "Content-Type": "application/json",
        }
      });
    } else {
      // 使用 POST 方法（现有逻辑）
      response = await fetch(`${ENDPOINT}/api/${command}`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify(args || {}),
      });
    }
    
    // if status is 405, it means the command is not allowed
    if (response.status === 405) {
      throw new Error(
        `Command ${command} is not allowed, maybe bili-shadowreplay is running in readonly mode or HTTP method mismatch`
      );
    }
    if (!response.ok) {
      const error = await response.json().catch(() => ({
        message: `HTTP ${response.status}`
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
      const config = await invoke("get_config") as any;
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

async function convertCoverSrc(coverPath: string, videoId?: number) {
  if (TAURI_ENV) {
    // 在客户端模式下，如果是base64数据URL，需要特殊处理
    if (coverPath && coverPath.startsWith("data:image/")) {
      // 对于base64数据，直接返回，让浏览器处理
      return coverPath;
    }
    // 对于文件路径（缩略图等），需要获取config来构建绝对路径
    try {
      const config = await invoke("get_config") as any;
      const absolutePath = `${config.output}/${coverPath}`;
      return tauri_convert(absolutePath);
    } catch (error) {
      console.error("Failed to get config for cover path conversion:", error);
      return tauri_convert(coverPath);
    }
  }
  
  // 如果是base64数据URL，使用专门的API端点
  if (coverPath && coverPath.startsWith("data:image/") && videoId) {
    return `${ENDPOINT}/api/image/${videoId}`;
  }
  
  // 普通文件路径
  return `${ENDPOINT}/output/${coverPath}`;
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
  convertCoverSrc,
  ENDPOINT,
  listen,
  open,
  log,
  close_window,
  onOpenUrl,
};
