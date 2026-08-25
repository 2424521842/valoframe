import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["tests/ui/**/*.test.tsx"],
    setupFiles: ["./vitest.setup.ts"],
    // Full-App mount tests render the whole workspace tree and routinely need several seconds.
    // On a many-core machine vitest starts one worker per core, so the default 5s timeout expired
    // for whichever files happened to share a core; the failures moved around between runs.
    testTimeout: 20_000,
    maxWorkers: 8,
  },
});
