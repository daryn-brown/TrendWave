import { check, Update, type DownloadEvent } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export { Update, relaunch };

/**
 * Ask the configured GitHub release endpoint whether a newer, signature-verified
 * build exists. Resolves to `null` when already up to date. Throws when the
 * endpoint is unreachable or returns an invalid manifest (e.g. while running in
 * `tauri dev`, where there is no installed bundle to update).
 */
export function checkForUpdate(): Promise<Update | null> {
  return check();
}

/**
 * Download and stage an update, reporting progress as a 0-100 percentage. The
 * new bundle is verified against the embedded public key before it is applied;
 * call `relaunch()` afterwards to boot into it.
 */
export async function downloadAndInstall(
  update: Update,
  onProgress: (pct: number) => void,
): Promise<void> {
  let total = 0;
  let received = 0;
  await update.downloadAndInstall((event: DownloadEvent) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? 0;
        onProgress(0);
        break;
      case "Progress":
        received += event.data.chunkLength;
        onProgress(total > 0 ? Math.min(100, Math.round((received / total) * 100)) : 0);
        break;
      case "Finished":
        onProgress(100);
        break;
    }
  });
}
