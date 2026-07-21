/**
 * Opens an http(s)/mailto/tel URL in the OS default handler instead of
 * navigating the webview. A bare `<a target="_blank">` click resolves to a
 * `window.open` new-window request that Tauri's webview (WebView2 on
 * Windows, WKWebView on macOS) does not have a handler for, so it silently
 * does nothing on both platforms — this goes through `tauri-plugin-opener`
 * instead, which shells out to the OS directly.
 */
import { openUrl } from "@tauri-apps/plugin-opener";

export function openExternalLink(url: string) {
  void openUrl(url);
}
