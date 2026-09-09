import { beforeEach, describe, expect, it, vi } from "vitest";

const isPermissionGranted = vi.fn(() => Promise.resolve(true));
const requestPermission = vi.fn(() => Promise.resolve("granted"));
const sendNotification = vi.fn();
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: () => isPermissionGranted(),
  requestPermission: () => requestPermission(),
  sendNotification: (opts: unknown) => sendNotification(opts),
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  error: () => Promise.resolve(),
  warn: () => Promise.resolve(),
}));

// The permission check is cached at module scope (by design — see notify.ts),
// so each test gets a fresh module instance rather than fighting the cache
// left over from a previous test.
async function freshNotify() {
  vi.resetModules();
  const [notify, toast] = await Promise.all([
    import("./notify"),
    import("../state/toastStore"),
  ]);
  return {
    notifyAssistant: notify.notifyAssistant,
    toastStore: toast.useToastStore,
  };
}

describe("notifyAssistant", () => {
  beforeEach(() => {
    isPermissionGranted.mockClear();
    requestPermission.mockClear();
    sendNotification.mockClear();
    isPermissionGranted.mockResolvedValue(true);
  });

  it("toasts and sends an OS notification with the given title/body", async () => {
    const { notifyAssistant, toastStore } = await freshNotify();

    notifyAssistant("Assistant replied", "Two to-dos are open.");

    expect(toastStore.getState().toasts).toHaveLength(1);
    expect(toastStore.getState().toasts[0]).toMatchObject({
      kind: "info",
      message: "Assistant replied",
      detail: "Two to-dos are open.",
    });

    await vi.waitFor(() => expect(sendNotification).toHaveBeenCalled());
    expect(sendNotification).toHaveBeenCalledWith({
      title: "Assistant replied",
      body: "Two to-dos are open.",
    });
  });

  it("truncates a long body for both the toast and the OS notification", async () => {
    const { notifyAssistant, toastStore } = await freshNotify();
    const long = "x".repeat(250);

    notifyAssistant("Assistant replied", long);

    const toast = toastStore.getState().toasts[0];
    expect(toast.detail?.length).toBe(200);
    expect(toast.detail?.endsWith("…")).toBe(true);

    await vi.waitFor(() => expect(sendNotification).toHaveBeenCalled());
    const sent = sendNotification.mock.calls[0][0] as { body: string };
    expect(sent.body.length).toBe(200);
  });

  it("never surfaces a failure sending the OS notification", async () => {
    const { notifyAssistant, toastStore } = await freshNotify();
    sendNotification.mockImplementation(() => {
      throw new Error("boom");
    });

    expect(() => notifyAssistant("Assistant replied", "hi")).not.toThrow();
    await vi.waitFor(() => expect(sendNotification).toHaveBeenCalled());
    // Still toasted even though the OS call failed.
    expect(toastStore.getState().toasts).toHaveLength(1);
  });

  it("skips the OS notification when permission is refused, but still toasts", async () => {
    isPermissionGranted.mockResolvedValue(false);
    requestPermission.mockResolvedValue("denied");
    const { notifyAssistant, toastStore } = await freshNotify();

    notifyAssistant("Assistant has a question", "confirm this?");

    expect(toastStore.getState().toasts).toHaveLength(1);
    await vi.waitFor(() => expect(isPermissionGranted).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });
});
