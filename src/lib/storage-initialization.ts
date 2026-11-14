import { invoke } from "./invoker";

export interface StorageInitializationStatus {
  loading: boolean;
  error: string | null;
}

/** A local status snapshot; never trigger a remote catalog refresh here. */
export function getStorageInitializationStatus() {
  return invoke<StorageInitializationStatus>("get_storage_initialization_status");
}
