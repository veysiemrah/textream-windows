import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath, URL } from "node:url";

const entry = (name: string) =>
  fileURLToPath(new URL(`./${name}.html`, import.meta.url));

export default defineConfig({
  plugins: [svelte()],
  // Tauri owns the terminal; clearing it would wipe the Rust build output.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  build: {
    target: "chrome110",
    rollupOptions: {
      input: { main: entry("index"), overlay: entry("overlay") },
    },
  },
});
