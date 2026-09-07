import { deserializeMessages, type ChatMessage } from "./messages";

const DATABASE = "bsr-agent";
const STORE = "conversations";
const KEY = "messages";

// Serialize loads, saves, and clears, including across component remounts.
let pending: Promise<unknown> = Promise.resolve();

function enqueue<T>(operation: () => Promise<T>): Promise<T> {
  const result = pending.then(operation);
  pending = result.catch(() => {});
  return result;
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
}

async function accessConversation(
  mode: IDBTransactionMode,
  messages?: ChatMessage[],
): Promise<unknown> {
  const database = await openDatabase();
  try {
    return await new Promise((resolve, reject) => {
      const transaction = database.transaction(STORE, mode);
      const store = transaction.objectStore(STORE);
      const request = mode === "readonly" ? store.get(KEY) : store.put(messages, KEY);
      // A successful request can still be followed by an aborted transaction.
      transaction.oncomplete = () => resolve(request.result);
      transaction.onabort = () => reject(transaction.error ?? new Error("会话保存失败"));
      transaction.onerror = () => reject(transaction.error);
    });
  } finally {
    database.close();
  }
}

export function loadConversation(): Promise<ChatMessage[]> {
  return enqueue(async () => {
    const stored = await accessConversation("readonly");
    if (stored !== undefined) return deserializeMessages(stored);

    const legacy = localStorage.getItem(KEY);
    if (legacy === null) return [];
    const messages = deserializeMessages(JSON.parse(legacy));
    await accessConversation("readwrite", messages);
    // Keep the old history until the new transaction has committed.
    localStorage.removeItem(KEY);
    return messages;
  });
}

export function saveConversation(messages: ChatMessage[]): Promise<void> {
  // Callers must pass plain data (a $state.snapshot for Svelte state).
  const snapshot = structuredClone(messages);
  return enqueue(async () => {
    await accessConversation("readwrite", snapshot);
    localStorage.removeItem(KEY);
  });
}
