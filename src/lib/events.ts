import { listen as tauriListen } from "@tauri-apps/api/event";
import { io } from "socket.io-client";
import type { DanmuEntry } from "./interface";

type Listener = (payload: unknown) => void;
export type AppEvent<T> = { payload: T };
export type StateChangeEvent =
  | "videos-changed" | "tasks-changed" | "recorders-changed" | "records-changed" | "state-resync";
export type EventPayload<E extends string> =
  E extends `progress-update:${string}` ? { id: string; content: string } :
  E extends `progress-finished:${string}` ? { id: string; success: boolean; message: string } :
  E extends "danmu" ? DanmuEntry & { room: string } :
  E extends StateChangeEvent ? null : unknown;

/** All event types share one transport, dispatch path and subscription lifecycle. */
export function createEventClient({ tauri, endpoint }: { tauri: boolean; endpoint: string }) {
  const listeners = new Map<string, Set<Listener>>();

  function deliver<T>(event: string, callback: (event: AppEvent<T>) => void, message: AppEvent<T>) {
    try {
      callback(message);
    } catch (error) {
      console.error(`[Events] Listener error for ${event}:`, error);
    }
  }

  function dispatch(event: string, payload: unknown) {
    for (const callback of [...(listeners.get(event) || [])]) callback(payload);
  }

  const socket = tauri ? null : io(`${endpoint}/ws`, {
    transports: ["websocket", "polling"],
    autoConnect: true,
    reconnection: true,
  });

  socket?.on("connect", () => {
    // A snapshot repairs server events missed before connecting or reconnecting.
    dispatch("state-resync", null);
  });
  socket?.on("connect_error", (error) => {
    console.error("[Events] Connection error:", error);
  });
  socket?.onAny((event: string, payload: unknown) => {
    // Older servers used unscoped progress names. Normalize them at the edge.
    if ((event === "progress-update" || event === "progress-finished") &&
      payload && typeof payload === "object" && "id" in payload && typeof payload.id === "string") {
      event = `${event}:${payload.id}`;
    }
    dispatch(event, payload);
  });

  function listen<E extends string>(event: E, callback: (event: AppEvent<EventPayload<E>>) => void): Promise<() => void>;
  function listen<T>(event: string, callback: (event: AppEvent<T>) => void): Promise<() => void>;
  async function listen<T>(event: string, callback: (event: AppEvent<T>) => void) {
    if (tauri) return tauriListen<T>(event, (message) => deliver(event, callback, message));

    // Each registration gets its own identity, even for the same callback.
    const listener: Listener = (payload) => deliver(event, callback, { payload: payload as T });
    let callbacks = listeners.get(event);
    if (!callbacks) {
      callbacks = new Set();
      listeners.set(event, callbacks);
    }
    callbacks.add(listener);
    return () => {
      callbacks.delete(listener);
      if (callbacks.size === 0) listeners.delete(event);
    };
  }

  return { listen };
}
