import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["tests/ui/**/*.test.tsx"],
    setupFiles: ["./vitest.setup.ts"],
    // Full-App mount tests render the whole workspace tree behind React.lazy and need well over
    // the default 5s on a loaded machine. The async wait budget is raised in vitest.setup.ts.
    testTimeout: 20_000,
  },
});
