import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

// Graph editor and component tests (jsdom). The plain logic tests in
// tests/*.test.ts run with node's own test runner (see the "test" script).
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    include: ['tests/flow/**/*.test.{ts,tsx}'],
    setupFiles: ['tests/flow/setup.ts'],
    css: false,
  },
})
