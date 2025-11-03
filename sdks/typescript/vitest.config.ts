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
    include: ['tests/unit/**/*.test.ts'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      exclude: ['src/proto/**', 'dist/**', 'tests/**'],
    },
  },
});
