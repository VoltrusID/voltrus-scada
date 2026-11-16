export interface BrandingSettings {
  title: string
  subtitle: string
  logo: string
  /** Accent colour as `#rgb` or `#rrggbb`; empty or missing keeps the Voltrus green. */
  accent?: string
}

export const DEFAULT_BRANDING: BrandingSettings = {
  title: 'Voltrus',
  subtitle: 'Realtime Monitor',
  logo: '',
}

/** The Voltrus green of the Sign In button. */
export const DEFAULT_ACCENT = '#22c55e'

const HEX_COLOR = /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i

/** The accent colour to paint with: the brand's when it is a valid hex colour, else the default. */
export function brandAccent(branding: Partial<BrandingSettings> | null | undefined): string {
  const accent = typeof branding?.accent === 'string' ? branding.accent.trim() : ''
  return HEX_COLOR.test(accent) ? accent : DEFAULT_ACCENT
}

/**
 * Text colour on the accent: white on the default green (as before), and on
 * a brand accent white or near-black, whichever reads better.
 */
export function brandAccentText(branding: Partial<BrandingSettings> | null | undefined): string {
  const accent = brandAccent(branding)
  if (accent === DEFAULT_ACCENT) return '#fff'
  const hex = accent.length === 4 ? accent.slice(1).split('').map(c => c + c).join('') : accent.slice(1)
  const [r, g, b] = [0, 2, 4].map(i => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  })
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b
  // Contrast against white vs against #09090b (luminance ~0.003).
  return (1.05 / (luminance + 0.05)) >= ((luminance + 0.05) / 0.053) ? '#fff' : '#09090b'
}

/** The browser tab title: the brand title, else "Voltrus". */
export function brandDocumentTitle(branding: Partial<BrandingSettings> | null | undefined): string {
  const title = typeof branding?.title === 'string' ? branding.title.trim() : ''
  return title || DEFAULT_BRANDING.title
}

/**
 * The favicon to use for a brand logo, or null to keep the Voltrus icons.
 * Only image data: URLs and http(s) or same-origin paths are used.
 */
export function brandFavicon(branding: Partial<BrandingSettings> | null | undefined): string | null {
  const logo = typeof branding?.logo === 'string' ? branding.logo.trim() : ''
  if (!logo) return null
  if (/^data:image\//i.test(logo)) return logo
  if (/^https?:\/\//i.test(logo)) return logo
  if (logo.startsWith('/') && !logo.startsWith('//')) return logo
  return null
}

const ORIGINAL_HREF = 'data-voltrus-href'
const ORIGINAL_TYPE = 'data-voltrus-type'

/**
 * Apply the brand to the browser tab: document title and favicon. Restores
 * the Voltrus icons from index.html when the brand has no usable logo.
 */
export function applyBrandToDocument(doc: Document, branding: Partial<BrandingSettings> | null | undefined): void {
  doc.title = brandDocumentTitle(branding)
  const favicon = brandFavicon(branding)
  doc.querySelectorAll<HTMLLinkElement>('link[rel~="icon"]').forEach(link => {
    if (!link.hasAttribute(ORIGINAL_HREF)) {
      link.setAttribute(ORIGINAL_HREF, link.getAttribute('href') ?? '')
      link.setAttribute(ORIGINAL_TYPE, link.getAttribute('type') ?? '')
    }
    const href = favicon ?? link.getAttribute(ORIGINAL_HREF) ?? ''
    if (link.getAttribute('href') !== href) link.setAttribute('href', href)
    // index.html says image/png; a brand logo may be SVG or JPEG.
    const type = favicon ? '' : link.getAttribute(ORIGINAL_TYPE) ?? ''
    if (type) link.setAttribute('type', type)
    else link.removeAttribute('type')
  })
}
