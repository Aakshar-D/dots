import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Tauri loads the dev server at a fixed port (apps/dots-app/tauri.conf.json `devUrl`).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "dist", target: "es2022" },
  test: { environment: "node" },
});
