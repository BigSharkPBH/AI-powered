import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  publicDir: false,
  envPrefix: ["TAURI_ENV_"],
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    watch: {
      // Rust 编译时会锁定 PDB 文件，由 Tauri 负责监听后台目录。
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist-tauri-ui",
    emptyOutDir: true,
    target: "chrome105",
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    passWithNoTests: true,
  },
});
