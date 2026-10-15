import { invoke, isTauri } from "@tauri-apps/api/core";
export type Request = { action: string } & Record<string, unknown>;
export async function requestNative(request: Request): Promise<unknown> {
  if (!isTauri()) throw new Error("Open the desktop app to scan your disks.");
  return invoke("request", { request });
}
