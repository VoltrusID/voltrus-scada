import './i18n'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import './index.css'
import 'react-grid-layout/css/styles.css'
import 'react-resizable/css/styles.css'
import App from './App'
import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'
import { toast } from './components/Toast'
import { isSignatureCancelled } from './signature/logic'
import { isPasswordChangeRequiredError, isSessionExpiredError } from './session'
import { installChunkReload } from './chunkReload'
import { ErrorBoundary } from './components/ErrorBoundary'

// A redeploy while this page is open leaves it asking for chunks that are gone.
installChunkReload()

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      gcTime: 5 * 60_000,
      refetchOnWindowFocus: false,
      retry: 1,
    },
    mutations: {
      onError: (err: any) => {
        // Cancelling a signature prompt abandons the action quietly.
        if (isSignatureCancelled(err)) return
        // The change-password screen replaces the app: no toast.
        if (isPasswordChangeRequiredError(err)) return
        // The login screen says why the session ended.
        if (isSessionExpiredError(err)) return
        toast.error(err.message || 'Request failed')
      },
    },
  },
})

queryClient.getQueryCache().config.onError = (error: any) => {
  if (isPasswordChangeRequiredError(error) || isSessionExpiredError(error)) return
  toast.error(error.message || 'Request failed')
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App><ErrorBoundary fullScreen><RouterProvider router={router} /></ErrorBoundary></App>
    </QueryClientProvider>
  </StrictMode>
)

// Register service worker for push notifications
import { registerServiceWorker } from './sw-register'
registerServiceWorker()
