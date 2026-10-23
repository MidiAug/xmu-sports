import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    // 固定 IPv4：Vite 默认可能只绑 [::1]，WebView 解析 localhost→127.0.0.1 会长时间 SYN 超时
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    // 勿监视 Rust 产物/侧车；否则 chokidar 扫 src-tauri/target 会把 Vite 拖到数秒/次请求
    watch: {
      ignored: [
        "**/src-tauri/**",
        "**/archive/**",
        "**/reference/**",
        "**/captures/**",
        "**/artifacts/**",
        "**/.codegraph/**",
        "**/node_modules/**",
      ],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "esnext",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
