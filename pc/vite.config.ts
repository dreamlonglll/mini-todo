import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { resolve } from "path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
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
    // 显式绑 127.0.0.1：默认的 false 会让 Vite 绑到 "localhost"，而 Windows 上
    // localhost 优先解析成 ::1，Node 17+ 按首个解析结果绑定，结果只监听 IPv6。
    // tauri dev 探测 http://localhost:1420/ 走 IPv4 会被拒，卡在
    // "Waiting for your frontend dev server to start" 不动
    host: host || "127.0.0.1",
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
