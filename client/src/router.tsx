import {
  createRootRoute,
  createRoute,
  createRouter,
} from '@tanstack/react-router'
import { AppInner } from './App'

// Voltrus is a single-shell SPA: AppInner (the root component) renders the
// whole dashboard chrome and branches on the active route itself. Child
// routes exist to make each destination a real URL + history entry (so
// deep-linking and the browser back/forward buttons work). Their components
// render nothing — AppInner owns all rendering, driven by useNavigate /
// useRouterState. See client/src/App.tsx for the routing-aware state.
const None = () => null

export const rootRoute = createRootRoute({
  component: AppInner,
  // Optional search schema (defaults resolved in App.tsx via ??). Keeping
  // fields optional avoids making `search` mandatory on every navigate and
  // lets setters merge with `(prev) => ({ ...prev, x })` updaters.
  validateSearch: (input: Record<string, unknown>): { screen?: string; view?: 'cards' | 'canvas' | 'twin' | 'scada'; mode?: 'live' | 'history'; tab?: string } => {
    const out: { screen?: string; view?: 'cards' | 'canvas' | 'twin' | 'scada'; mode?: 'live' | 'history'; tab?: string } = {}
    if (typeof input.screen === 'string') out.screen = input.screen
    if (input.view === 'canvas' || input.view === 'twin' || input.view === 'scada') out.view = input.view
    if (input.mode === 'history') out.mode = 'history'
    if (typeof input.tab === 'string') out.tab = input.tab
    return out
  },
})

const indexRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: None })
const sensorRoute = createRoute({ getParentRoute: () => rootRoute, path: '/sensor/$id', component: None })
const systemRoute = createRoute({ getParentRoute: () => rootRoute, path: '/system', component: None })
const flowsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/flows', component: None })
const tagsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/tags', component: None })
const assetsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/assets', component: None })
const scriptsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/scripts', component: None })
const dataRoute = createRoute({ getParentRoute: () => rootRoute, path: '/data', component: None })
const alarmsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/alarms', component: None })
const commandsRoute = createRoute({ getParentRoute: () => rootRoute, path: '/commands', component: None })
const shiftRoute = createRoute({ getParentRoute: () => rootRoute, path: '/shift', component: None })

const routeTree = rootRoute.addChildren([
  indexRoute,
  sensorRoute,
  systemRoute,
  flowsRoute,
  tagsRoute,
  assetsRoute,
  scriptsRoute,
  dataRoute,
  alarmsRoute,
  commandsRoute,
  shiftRoute,
])

export const router = createRouter({
  routeTree,
  defaultPreload: false,
  scrollRestoration: false,
})

// Type registration for fully-typed navigate/useSearch/useParams.
declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
