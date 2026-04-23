import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./web/tests",
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  timeout: 30_000,
  use: {
    baseURL: "http://localhost:5173",
    trace: "retain-on-failure",
    permissions: ["clipboard-read", "clipboard-write"],
  },
  projects: [
    {
      name: "chromium",
      use: { browserName: "chromium" },
    },
  ],
  webServer: {
    command: "npx vite --port 5173 --strictPort",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
