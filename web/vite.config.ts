import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// 构建产物输出到 web/dist，由 Rust 侧 rust-embed 编译进二进制。
// dev 时把 /v1 代理到本地 Hub（triskelion 默认 127.0.0.1:8787）。
export default defineConfig({
  // 相对基准路径：构建产物用 ./assets/... 引用，便于整体置于 nginx 子路径后转发。
  base: "./",
  plugins: [react(), tailwindcss()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  server: {
    proxy: {
      "/v1": "http://127.0.0.1:8787",
    },
  },
});
