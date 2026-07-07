/**
 * Host-platform detection for the frontend.
 *
 * Single source of truth so chrome that differs per OS (the macOS native menu
 * vs. the Windows in-app title bar / settings gear / window controls) all read
 * the same flags. `navigator.platform` is deprecated but still the most reliable
 * signal inside WebView2 / WKWebView; we fall back to the user-agent string.
 *
 * Computed once at module load — the platform never changes within a session.
 */

const haystack = navigator.platform || navigator.userAgent;

// Word-precise patterns: a bare /win/i would also match jsdom's
// "Mozilla/5.0 (darwin) …" user agent and mis-detect tests as Windows.
export const isMac = /mac/i.test(haystack);
export const isWindows = /windows|win32|win64/i.test(haystack);
