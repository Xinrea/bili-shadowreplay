import { listen as tauriListen } from "@tauri-apps/api/event";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { io } from "socket.io-client";
import type { DanmuEntry, ProgressUpdate } from "./interface";

type Listener = (payload: unknown) => void;
export type RoomTopic = { platform: string; room: string };
type RoomRegistration = { topic: RoomTopic; count: number; ready: Promise<void> };
const MAX_ROOM_TOPICS = 32;
const ROOM_ACK_TIMEOUT_MS = 5_000;
const ROOM_RETRY_INITIAL_MS = 1_000;
const ROOM_RETRY_MAX_MS = 30_000;
type RoomSync = { promise: Promise<void>; resolve: () => void; reject: (error: unknown) => void };

function roomEventName({ platform, room }: RoomTopic) {
  const hex = [...new TextEncoder().encode(room)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return `danmu:${platform}:${hex}`;
}
export type AppEvent<T> = { payload: T };
export type StateChangeEvent =
  | "videos-changed" | "tasks-changed" | "recorders-changed" | "records-changed" | "state-resync";
export type EventPayload<E extends string> =
  E extends `progress-update:${string}` ? ProgressUpdate :
  E extends `progress-finished:${string}` ? { id: string; success: boolean; message: string } :
  E extends "danmu" ? DanmuEntry & RoomTopic :
  E extends StateChangeEvent ? null : unknown;

/** All event types share one transport, dispatch path and subscription lifecycle. */
export function createEventClient({ tauri, endpoint }: { tauri: boolean; endpoint: string }) {
  const listeners = new Map<string, Set<Listener>>();
  const rooms = new Map<string, RoomRegistration>();
  let roomUpdates = Promise.resolve();
  let webRoomSync: RoomSync | null = null;
  let webRoomRevision = 0;
  let webRoomFlight: { revision: number } | null = null;
  let webRoomRetryTimer: ReturnType<typeof setTimeout> | undefined;
  let webRoomRetryDelay = ROOM_RETRY_INITIAL_MS;

  function cancelWebRoomRetry(resetDelay = true) {
    if (webRoomRetryTimer !== undefined) clearTimeout(webRoomRetryTimer);
    webRoomRetryTimer = undefined;
    if (resetDelay) webRoomRetryDelay = ROOM_RETRY_INITIAL_MS;
  }

  function scheduleWebRoomRetry() {
    if (!socket.connected || rooms.size === 0 || webRoomRetryTimer !== undefined) return;
    const delay = webRoomRetryDelay;
    webRoomRetryDelay = Math.min(delay * 2, ROOM_RETRY_MAX_MS);
    webRoomRetryTimer = setTimeout(() => {
      webRoomRetryTimer = undefined;
      void syncRooms(false).catch(reportRoomError);
    }, delay);
  }

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

  function flushWebRooms() {
    if (!socket.connected || webRoomFlight) return;
    const flight = { revision: webRoomRevision };
    webRoomFlight = flight;
    // Read the current set when sending, not when an update was queued.
    const topics = [...rooms.values()].map(({ topic }) => topic);
    void socket.timeout(ROOM_ACK_TIMEOUT_MS).emitWithAck("event-rooms", topics).then((response) => {
      if (response?.ok !== true) throw new Error(response?.error || "Room subscription was rejected");
    }).then(() => finish(), (error) => finish({ error }));

    function finish(failure?: { error: unknown }) {
      // Disconnect/reconnect invalidates the flight, including its late ACK.
      if (webRoomFlight !== flight) return;
      webRoomFlight = null;
      if (!socket.connected) return;
      if (flight.revision !== webRoomRevision) {
        flushWebRooms();
        return;
      }
      const pending = webRoomSync;
      webRoomSync = null;
      if (!failure) {
        cancelWebRoomRetry();
        pending?.resolve();
      } else {
        // Established listeners no longer await setup. Keep their current set
        // alive and recover it even if reconnect acknowledgement fails.
        scheduleWebRoomRetry();
        pending?.reject(failure.error);
      }
    }
  }

  function syncRooms(resetRetryDelay = true): Promise<void> {
    if (tauri) {
      const topics = [...rooms.values()].map(({ topic }) => topic);
      // Serialize desktop commands so a late subscribe cannot undo an unlisten.
      roomUpdates = roomUpdates.catch(() => {}).then(() => tauriInvoke<void>("set_event_rooms", { rooms: topics }));
      return roomUpdates;
    }
    cancelWebRoomRetry(resetRetryDelay);
    // Readiness belongs to convergence of the current set, not to an individual
    // packet. One flight per connection serializes changes; a new connection
    // immediately starts a fresh flight without waiting for obsolete ACKs.
    webRoomRevision++;
    if (!webRoomSync) {
      let resolve!: () => void;
      let reject!: (error: unknown) => void;
      const promise = new Promise<void>((done, fail) => { resolve = done; reject = fail; });
      webRoomSync = { promise, resolve, reject };
    }
    const ready = webRoomSync.promise;
    flushWebRooms();
    return ready;
  }

  function reportRoomError(error: unknown) {
    console.error("[Events] Room subscription error:", error);
  }

  // Reloading a desktop webview replaces its old preview subscriptions.
  if (tauri) void syncRooms().catch(reportRoomError);

  socket?.on("connect", () => {
    webRoomFlight = null;
    void syncRooms().catch(reportRoomError);
    // A snapshot repairs server events missed before connecting or reconnecting.
    dispatch("state-resync", null);
  });
  socket?.on("disconnect", () => {
    webRoomFlight = null;
    cancelWebRoomRetry();
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

  function listen<E extends string>(event: E, callback: (event: AppEvent<EventPayload<E>>) => void, room?: RoomTopic, signal?: AbortSignal): Promise<() => void>;
  function listen<T>(event: string, callback: (event: AppEvent<T>) => void, room?: RoomTopic, signal?: AbortSignal): Promise<() => void>;
  async function listen<T>(event: string, callback: (event: AppEvent<T>) => void, room?: RoomTopic, signal?: AbortSignal) {
    if (signal?.aborted) throw new Error("Event subscription was aborted");
    if (event === "danmu" && !room) throw new Error("Danmu subscriptions require a platform and room");
    if (room && event !== "danmu") throw new Error("Only danmu events use room subscriptions");
    const name = room ? roomEventName(room) : event;
    let unlisten: () => void;
    if (tauri) {
      // Listen locally before asking the producer to start forwarding.
      unlisten = await tauriListen<T>(name, (message) => deliver(event, callback, message), room ? {
        target: { kind: "WebviewWindow", label: getCurrentWebviewWindow().label },
      } : undefined);
    } else {

      // Each registration gets its own identity, even for the same callback.
      const listener: Listener = (payload) => deliver(event, callback, { payload: payload as T });
      let callbacks = listeners.get(name);
      if (!callbacks) {
        callbacks = new Set();
        listeners.set(name, callbacks);
      }
      callbacks.add(listener);
      unlisten = () => {
        callbacks.delete(listener);
        if (callbacks.size === 0) listeners.delete(name);
      };
    }

    let registration: RoomRegistration | undefined;
    if (room) {
      registration = rooms.get(name);
      if (!registration) {
        if (rooms.size >= MAX_ROOM_TOPICS) {
          unlisten();
          throw new Error(`At most ${MAX_ROOM_TOPICS} room topics may be subscribed`);
        }
        registration = { topic: { ...room }, count: 0, ready: Promise.resolve() };
        rooms.set(name, registration);
      }
      registration.count++;
      if (registration.count === 1) registration.ready = syncRooms();
    }
    let stopped = false;
    let onAbort: (() => void) | undefined;
    const stop = () => {
      if (stopped) return;
      stopped = true;
      if (onAbort) signal?.removeEventListener("abort", onAbort);
      unlisten();
      if (registration && --registration.count === 0 && rooms.get(name) === registration) {
        rooms.delete(name);
        void syncRooms().catch(reportRoomError);
      }
    };
    try {
      if (signal) {
        const aborted = new Promise<void>((_, reject) => {
          onAbort = () => { stop(); reject(new Error("Event subscription was aborted")); };
          signal.addEventListener("abort", onAbort, { once: true });
          if (signal.aborted) onAbort();
        });
        await Promise.race([registration?.ready ?? Promise.resolve(), aborted]);
      } else {
        await registration?.ready;
      }
      return stop;
    } catch (error) {
      stop();
      throw error;
    }
  }

  return { listen };
}
