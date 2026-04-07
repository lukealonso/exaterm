import { defineConfig } from "@playwright/test";
import path from "path";

const repoRoot = path.resolve(__dirname, "../..");
const webBin = path.join(repoRoot, "target/debug/exaterm-web");
const testPort = 19742;

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  workers: 1,
  use: {
    baseURL: `http://127.0.0.1:${testPort}`,
    headless: true,
    viewport: { width: 1920, height: 1080 },
  },
  webServer: {
    command: `EXATERM_RUNTIME_DIR=/tmp/exaterm-e2e-${process.pid} ${webBin} --port ${testPort}`,
    port: testPort,
    timeout: 15_000,
    reuseExistingServer: false,
  },
});
