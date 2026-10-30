import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 开发时固定端口，避免窗口指向错误地址
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "dist", target: "chrome110", emptyOutDir: true },
});
