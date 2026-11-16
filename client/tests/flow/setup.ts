import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

afterEach(() => cleanup())

// jsdom lacks the layout APIs React Flow measures with.
class RO { observe() {} unobserve() {} disconnect() {} }
const g = globalThis as Record<string, unknown>
g.ResizeObserver ??= RO
g.DOMMatrixReadOnly ??= class { m22 = 1 }
if (!window.matchMedia) {
  window.matchMedia = ((q: string) => ({
    matches: false, media: q, onchange: null,
    addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}, dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia
}

// styles.ts probes a 2D canvas for colour parsing; jsdom has none.
HTMLCanvasElement.prototype.getContext = (() => null) as unknown as HTMLCanvasElement["getContext"]
