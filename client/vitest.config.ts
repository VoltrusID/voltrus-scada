import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

const root = path.dirname(fileURLToPath(import.meta.url))

// Graph editor and component tests (jsdom). The plain logic tests in
// tests/*.test.ts run with node's own test runner (see the "test" script).
export default defineConfig({
  plugins: [react()],
  // Same aliases as vite.config.ts, with the Community edition stub.
  resolve: {
    alias: {
      '@voltrus/core': path.join(root, 'src'),
      '@voltrus/enterprise-client': path.join(root, 'src/edition/community.ts'),
    },
  },
  test: {
    environment: 'jsdom',
    include: ['tests/flow/**/*.test.{ts,tsx}'],
    setupFiles: ['tests/flow/setup.ts'],
    css: false,
  },
})
