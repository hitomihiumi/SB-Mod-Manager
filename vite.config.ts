import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri serves the frontend from a fixed port and needs a predictable failure
// if it is taken, rather than silently moving to another one.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  build: {
    target: "es2022",
    // Keep the shipped bundle small; the app has no need for legacy output.
    minify: "esbuild",
    sourcemap: false,
  },
});
