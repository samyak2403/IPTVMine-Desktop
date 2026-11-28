import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

function tvClassicScriptPlugin(): Plugin {
  return {
    name: "tv-classic-script",
    transformIndexHtml(html) {
      return html
        .replace(/<script\s+type="module"\s+crossorigin\s+/g, "<script defer ")
        .replace(/\s+crossorigin(?=>|\s)/g, "");
    },
  };
}

export default defineConfig(({ mode }) => {
  const isTv = mode === "tv";
  return {
    base: isTv ? "./" : "/",
    plugins: isTv ? [react(), tvClassicScriptPlugin()] : [react()],
    clearScreen: false,
    server: {
      port: 5173,
      strictPort: true,
    },
    build: isTv
      ? {
          outDir: "dist-tv",
          target: "es2020",
          rollupOptions: {
            output: {
              format: "iife",
              inlineDynamicImports: true,
              entryFileNames: "assets/app.js",
              assetFileNames: "assets/[name]-[hash][extname]",
            },
          },
        }
      : undefined,
  };
});
