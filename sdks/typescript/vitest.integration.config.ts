import { defineConfig } from 'vitest/config';
import path from 'path';

export default defineConfig({
  resolve: {
    alias: [
      {
        find: /^@norikv\/client\/(.*)$/,
        replacement: path.resolve(__dirname, './src/$1.ts'),
      },
      {
        find: '@norikv/client',
        replacement: path.resolve(__dirname, './src/index.ts'),
      },
    ],
  },
  test: {
    globals: true,
    environment: 'node',
    include: ['tests/integration/**/*.test.ts'],
    // Longer timeouts for integration tests (especially e2e with real server)
    testTimeout: 30000,
    hookTimeout: 30000,
    teardownTimeout: 10000,
    // Run tests sequentially to avoid port conflicts with test servers
    pool: 'forks',
    poolOptions: {
      forks: {
        singleFork: true,
      },
    },
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      exclude: ['src/proto/**', 'dist/**', 'tests/**'],
    },
  },
});
