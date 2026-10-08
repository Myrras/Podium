/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { configDefaults } from "vitest/config";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Vitest: component/integration tests run in a jsdom DOM with React Testing
  // Library; the setup file registers jest-dom matchers and resets the DOM and
  // module mocks between tests so each case is deterministic and isolated.
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    // Off elsewhere; AssistantPane.test.tsx imports its stylesheet `?raw`,
    // which `css: false` would stub to an empty string.
    css: { include: [/AssistantPane\.module\.css/] },
    clearMocks: true,
    restoreMocks: true,
    // `exclude` replaces Vitest's defaults rather than extending them, so
    // spread `configDefaults.exclude` (node_modules, dist, ...) and add the
    // worktree dirs — other agents' sibling worktrees live under
    // `.claude/worktrees/**` or Podium's own `.podium/worktrees/**` with their
    // own `node_modules`, and picking up their test files pulls in a second
    // React copy (hook errors that have nothing to do with this repo's own
    // tests).
    exclude: [...configDefaults.exclude, "**/.claude/**", "**/.podium/**"],
  },

  build: {
    chunkSizeWarningLimit: 800,
    rollupOptions: {
      output: {
        // The heavy vendor trees each go in their own chunk, so the app entry
        // stays small and a dependency bump only invalidates its own chunk.
        // Matched by path because the weight is in transitive packages
        // (prosemirror-* under tiptap, micromark/mdast under react-markdown)
        // that naming the direct dependency would not pull along.
        manualChunks(id) {
          if (!id.includes("node_modules")) return;
          if (id.includes("@xterm")) return "xterm";
          if (
            id.includes("@tiptap") ||
            id.includes("tiptap-markdown") ||
            id.includes("prosemirror")
          )
            return "editor";
          if (
            id.includes("react-markdown") ||
            id.includes("remark") ||
            id.includes("micromark") ||
            id.includes("mdast") ||
            id.includes("hast") ||
            id.includes("unist") ||
            id.includes("unified")
          )
            return "markdown";
          if (id.includes("lucide-react")) return "icons";
          if (id.includes("/react-dom/") || id.includes("/react/"))
            return "react";
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
