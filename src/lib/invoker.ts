import { invoke as tauri_invoke } from "@tauri-apps/api/core";

// HTTP 客户端配置
function getApiBaseUrl() {
  // get from local storage
  return localStorage.getItem("api_base_url");
}

async function invoke<T>(
  command: string,
  args?: Record<string, any>
): Promise<T> {
  try {
    const API_BASE_URL = getApiBaseUrl();
    if (!API_BASE_URL) {
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

    if (!response.ok) {
      const error = await response.json();
      throw new Error(error.message || `HTTP error: ${response.status}`);
    }

    const data = await response.json();
    return data as T;
  } catch (error) {
    // 将 HTTP 错误转换为 Tauri 风格的错误
    throw new Error(`Failed to invoke ${command}: ${error.message}`);
  }
}

export { invoke };
