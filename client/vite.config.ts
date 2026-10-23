import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

const root = path.dirname(fileURLToPath(import.meta.url))

// `@voltrus/core/*` resolves to this client's `src`. The enterprise module
// imports core through it, so the module works in any checkout layout. The
// enterprise vite config passes VOLTRUS_CORE_CLIENT_SRC explicitly; a plain
// core build uses ./src.
const coreSrc = path.resolve(root, process.env.VOLTRUS_CORE_CLIENT_SRC ?? 'src')

// `@voltrus/enterprise-client` is the Community stub unless
// VOLTRUS_ENTERPRISE_CLIENT points at the private module's entry file.
const enterpriseClient = process.env.VOLTRUS_ENTERPRISE_CLIENT
  ? path.resolve(process.env.VOLTRUS_ENTERPRISE_CLIENT)
  : path.join(coreSrc, 'edition/community.ts')

/**
 * Files of the enterprise module live outside this project, so Node-style
 * lookup from their directory cannot find this project's node_modules (react,
 * react-query...). Resolve their bare imports from the core client instead,
 * which also guarantees a single copy of every dependency.
 */
function resolveFromCore(): Plugin {
  const anchor = path.join(root, 'index.html')
  return {
    name: 'voltrus-resolve-from-core',
    enforce: 'pre',
    async resolveId(source, importer, options) {
      const isBare = /^[a-zA-Z@]/.test(source)
      const fromEnterprise = importer !== undefined
        && !importer.startsWith(root + path.sep)
        && !importer.includes(`${path.sep}node_modules${path.sep}`)
      if (!isBare || !fromEnterprise) return null
      return this.resolve(source, anchor, { ...options, skipSelf: true })
    },
  }
}

export default defineConfig({
  // Explicit so the config also works when loaded from the enterprise vite config.
  root,
  plugins: [resolveFromCore(), react()],
  resolve: {
    alias: {
      '@voltrus/core': coreSrc,
      '@voltrus/enterprise-client': enterpriseClient,
    },
  },
  server: {
    fs: {
      allow: [root, path.dirname(coreSrc), path.dirname(enterpriseClient)],
    },
    proxy: {
      // Server serves the built client same-origin in production, so every
      // API path (/api/v1, /api/license, /api/simulation, ...) must reach it
      // in dev too. ws: true forwards the /api/v1/ws live-data socket.
      '/api': {
        target: 'http://localhost:3100',
        ws: true,
      }
    }
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
    minify: true,
    rolldown: {
      drop: ['console', 'debugger'],
    },
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('/node_modules/three/')) return 'three'
          if (id.includes('/node_modules/react/') || id.includes('/node_modules/react-dom/')) return 'vendor-react'
          if (id.includes('/node_modules/lightweight-charts/')) return 'lightweight-charts'
        },
      },
    },
  }
})
