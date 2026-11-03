import { defineConfig } from "@playwright/test";

const host = "127.0.0.1";
const port = 4173;
const baseURL = `http://${host}:${port}`;

export default defineConfig({
  testDir: "./tests",
  use: {
    baseURL,
    viewport: { width: 420, height: 700 },
    launchOptions: {
      executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH,
    },
  },
  webServer: {
    command: `pnpm build && pnpm preview --host ${host} --port ${port} --strictPort`,
    url: baseURL,
  },
});
