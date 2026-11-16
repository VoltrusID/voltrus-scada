import test from 'node:test'
import assert from 'node:assert/strict'
import {
  DEFAULT_ACCENT,
  DEFAULT_BRANDING,
  applyBrandToDocument,
  brandAccent,
  brandAccentText,
  brandDocumentTitle,
  brandFavicon,
} from '../src/edition/branding.ts'

test('defaults are unchanged without branding (Core edition, 403/404)', () => {
  for (const b of [null, undefined, DEFAULT_BRANDING, {}]) {
    assert.equal(brandAccent(b), DEFAULT_ACCENT)
    assert.equal(brandAccentText(b), '#fff')
    assert.equal(brandDocumentTitle(b), 'Voltrus')
    assert.equal(brandFavicon(b), null)
  }
})

test('the accent is used only when it is a hex colour', () => {
  assert.equal(brandAccent({ accent: '#1d4ed8' }), '#1d4ed8')
  assert.equal(brandAccent({ accent: ' #0AF ' }), '#0AF')
  for (const bad of ['red', '1d4ed8', '#12', 'url(x)', '#1d4ed8;color:red']) {
    assert.equal(brandAccent({ accent: bad }), DEFAULT_ACCENT, bad)
  }
  assert.equal(brandAccentText({ accent: '#1d4ed8' }), '#fff')
  assert.equal(brandAccentText({ accent: '#fde047' }), '#09090b')
})

test('the tab title is the brand title', () => {
  assert.equal(brandDocumentTitle({ title: '  Pabrik Gula Djatiroto — Ruang Kontrol 🏭 ' }), 'Pabrik Gula Djatiroto — Ruang Kontrol 🏭')
  assert.equal(brandDocumentTitle({ title: '   ' }), 'Voltrus')
})

test('the favicon is the brand logo when it is an image URL', () => {
  assert.equal(brandFavicon({ logo: 'data:image/png;base64,AAAA' }), 'data:image/png;base64,AAAA')
  assert.equal(brandFavicon({ logo: 'https://cdn.example.com/logo.svg' }), 'https://cdn.example.com/logo.svg')
  assert.equal(brandFavicon({ logo: '/uploads/logo.png' }), '/uploads/logo.png')
  for (const bad of ['javascript:alert(1)', 'data:text/html,x', '//evil.example/x.png', 'logo.png']) {
    assert.equal(brandFavicon({ logo: bad }), null, bad)
  }
})

/** Just enough of a <link> and a Document for applyBrandToDocument. */
function fakeDoc() {
  const link = (href: string, type?: string) => {
    const attrs = new Map<string, string>([['href', href]])
    if (type) attrs.set('type', type)
    return {
      getAttribute: (k: string) => attrs.get(k) ?? null,
      setAttribute: (k: string, v: string) => { attrs.set(k, v) },
      hasAttribute: (k: string) => attrs.has(k),
      removeAttribute: (k: string) => { attrs.delete(k) },
    }
  }
  const links = [link('/favicon-32.png', 'image/png'), link('/favicon.ico')]
  const doc = { title: 'Voltrus', querySelectorAll: () => links }
  return { doc: doc as unknown as Document, links }
}

test('applying a brand sets title and favicon, and clearing it restores Voltrus', () => {
  const { doc, links } = fakeDoc()
  applyBrandToDocument(doc, { title: 'Plant A', subtitle: '', logo: 'data:image/svg+xml;base64,PHN2Zz4=' })
  assert.equal(doc.title, 'Plant A')
  for (const l of links) {
    assert.equal(l.getAttribute('href'), 'data:image/svg+xml;base64,PHN2Zz4=')
    assert.equal(l.getAttribute('type'), null)
  }
  applyBrandToDocument(doc, null)
  assert.equal(doc.title, 'Voltrus')
  assert.equal(links[0].getAttribute('href'), '/favicon-32.png')
  assert.equal(links[0].getAttribute('type'), 'image/png')
  assert.equal(links[1].getAttribute('href'), '/favicon.ico')
  assert.equal(links[1].getAttribute('type'), null)
})
