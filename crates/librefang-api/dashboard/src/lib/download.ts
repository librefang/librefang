// Save an in-memory Blob as a file through a temporary object-URL anchor.

/** How long the object URL outlives the click, so the browser has dereferenced it before it is revoked. */
export const DOWNLOAD_REVOKE_DELAY_MS = 1000;

/**
 * Trigger a browser download of `blob` named `filename`.
 *
 * The anchor is attached to the document for the click because some browsers ignore `click()` on a detached anchor.
 * The object URL is revoked after a delay rather than right after `click()`: revoking synchronously can cancel the download in Safari and WebKit-based webviews (the Tauri desktop build), leaving no file and no error.
 */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), DOWNLOAD_REVOKE_DELAY_MS);
}
