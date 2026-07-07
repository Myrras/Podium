import { describe, expect, it } from "vitest";

import { base64ToBytes, bytesToBase64, encodeInput } from "./termProtocol";

describe("termProtocol", () => {
  it("round-trips arbitrary bytes", () => {
    const bytes = new Uint8Array([0, 1, 2, 127, 128, 255, 27, 91, 65]);
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes);
  });

  it("round-trips the empty payload", () => {
    expect(base64ToBytes(bytesToBase64(new Uint8Array(0)))).toEqual(
      new Uint8Array(0),
    );
  });

  it("handles large payloads across the chunking boundary", () => {
    const bytes = new Uint8Array(0x8000 * 2 + 17);
    for (let i = 0; i < bytes.length; i++) bytes[i] = i % 256;
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes);
  });

  it("encodes multibyte input as UTF-8 (btoa alone would throw)", () => {
    const decoded = base64ToBytes(encodeInput("héllo 🚀"));
    expect(new TextDecoder().decode(decoded)).toBe("héllo 🚀");
  });

  it("encodes control sequences byte-for-byte", () => {
    // ESC [ A (cursor up) — the kind of raw bytes xterm onData produces.
    const decoded = base64ToBytes(encodeInput("\x1b[A"));
    expect(Array.from(decoded)).toEqual([0x1b, 0x5b, 0x41]);
  });
});
