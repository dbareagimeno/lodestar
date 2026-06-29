import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Vite + Svelte 5. El build estático (`dist/`) lo sirve la fachada Tauri (E6).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "esnext", outDir: "dist" },
});
