import { defineConfig } from "vite";
export default defineConfig({
  base: "./",
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true },
  build: {
    outDir: "dist/ui",
    emptyOutDir: true,
    // xterm.js is one bundle with the app; this is a local desktop page.
    chunkSizeWarningLimit: 900,
    rollupOptions: {
      // lucide-react ships React Server Component directives that do not apply here.
      onwarn(warning, warn) {
        if (warning.code !== "MODULE_LEVEL_DIRECTIVE") warn(warning);
      },
    },
  },
});
