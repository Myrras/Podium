/**
 * Byte-level helpers for the terminal wire protocol.
 *
 * Terminal data crosses the IPC bridge as base64 (raw PTY bytes are not
 * JSON-safe). Encoding must be multibyte-safe: `btoa`/`atob` work on binary
 * strings, so text is first converted through `TextEncoder` — never pass
 * user input to `btoa` directly (it throws on code points > 0xFF).
 */

/** Encode raw bytes as base64 (chunked to avoid call-stack limits). */
export function bytesToBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

/** Decode base64 into raw bytes (suitable for `Terminal.write`). */
export function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/** Encode keyboard input (UTF-8 text) as base64 for `process_write`. */
export function encodeInput(text: string): string {
  return bytesToBase64(new TextEncoder().encode(text));
}
