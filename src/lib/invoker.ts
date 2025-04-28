import { invoke as tauri_invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { fetch as tauri_fetch } from "@tauri-apps/plugin-http";

const API_BASE_URL = `${__API_BASE_URL__}`;
const TAURI_ENV = API_BASE_URL === "";

async function invoke<T>(
  command: string,
  args?: Record<string, any>
): Promise<T> {
  if (command === "open_live") {
    console.log(args);
    // open new page to live_index.html
    window.open(
      `live_index.html?platform=${args.platform}&room_id=${args.roomId}&live_id=${args.liveId}`,
      "_blank"
    );
    return;
  }
  try {
    if (TAURI_ENV) {
      // using tauri invoke
      return await tauri_invoke<T>(command, args);
    }
    const response = await fetch(`${API_BASE_URL}/${command}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(args || {}),
    });

    // reponse might be
    // application/vnd.apple.mpegurl
    // application/octet-stream
    const content_type = response.headers.get("Content-Type");
    if (
      content_type === "application/vnd.apple.mpegurl" ||
      content_type === "application/octet-stream"
    ) {
      const arrayBuffer = await response.arrayBuffer();
      console.log(arrayBuffer);
      return Array.from(new Uint8Array(arrayBuffer)) as T;
    }

    if (!response.ok) {
      const error = await response.json();
      throw new Error(error.message || `HTTP error: ${response.status}`);
    }

    const resp = await response.json();
    return resp.data as T;
  } catch (error) {
    // 将 HTTP 错误转换为 Tauri 风格的错误
    throw new Error(`Failed to invoke ${command}: ${error.message}`);
  }
}

async function get(url: string) {
  if (TAURI_ENV) {
    return await tauri_fetch(url);
  }
  const response = await fetch(`${API_BASE_URL}/fetch`, {
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

export { invoke, get, set_title, TAURI_ENV };
