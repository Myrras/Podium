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
    css: false,
    clearMocks: true,
    restoreMocks: true,
    // `exclude` replaces Vitest's defaults rather than extending them, so
    // spread `configDefaults.exclude` (node_modules, dist, ...) and add
    // `.claude` — other agents' sibling worktrees can live under
    // `.claude/worktrees/**` with their own `node_modules`, and picking up
    // their test files pulls in a second React copy (hook errors that have
    // nothing to do with this repo's own tests).
    exclude: [...configDefaults.exclude, "**/.claude/**"],
  },

  build: {
    // xterm is comparatively large; keep it in its own cached vendor chunk.
    chunkSizeWarningLimit: 800,
    rollupOptions: {
      output: {
        manualChunks: {
          xterm: ["@xterm/xterm"],
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
