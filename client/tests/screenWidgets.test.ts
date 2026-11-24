import { test, afterEach } from 'node:test'
import assert from 'node:assert/strict'
import {
  getScreenWidget, initialConfigOf, registerScreenWidget, registerScreenWidgets, resolveScreenWidget,
  scadaSizeOf, dashboardSizeOf, screenWidgets, unregisterScreenWidget, UNAVAILABLE_WIDGET_MESSAGE,
  type ScreenWidgetDef,
} from '../src/edition/widgets.ts'
import { extractDashboardWidgets, withDashboardWidgets, widgetItemKey } from '../src/dashboardWidgets.ts'

const render = () => null
const camera: ScreenWidgetDef = {
  type: 'enterprise.camera', label: 'Camera', surfaces: ['dashboard', 'scada'],
  defaultSize: { w: 8, h: 6 }, defaultConfig: { camera_id: null, fit: 'cover' }, render,
}
const scadaOnly: ScreenWidgetDef = { type: 'acme.dial', label: 'Dial', surfaces: ['scada'], render }

afterEach(() => {
  for (const d of screenWidgets()) unregisterScreenWidget(d.type)
})

test('nothing is registered in a Community build', () => {
  assert.deepEqual(screenWidgets(), [])
  assert.deepEqual(screenWidgets('dashboard'), [])
  assert.equal(getScreenWidget('enterprise.camera'), undefined)
})

test('registered widgets are listed per surface, in registration order', () => {
  registerScreenWidget(camera)
  registerScreenWidget(scadaOnly)
  assert.deepEqual(screenWidgets().map(d => d.type), ['enterprise.camera', 'acme.dial'])
  assert.deepEqual(screenWidgets('dashboard').map(d => d.type), ['enterprise.camera'])
  assert.deepEqual(screenWidgets('scada').map(d => d.type), ['enterprise.camera', 'acme.dial'])
  assert.equal(getScreenWidget('acme.dial', 'dashboard'), undefined)
  assert.equal(getScreenWidget('acme.dial', 'scada'), scadaOnly)
})

test('registering a type again replaces it in place', () => {
  registerScreenWidget(camera)
  registerScreenWidget(scadaOnly)
  const v2 = { ...camera, label: 'Camera v2' }
  assert.equal(registerScreenWidget(v2), camera)
  assert.deepEqual(screenWidgets().map(d => d.label), ['Camera v2', 'Dial'])
})

test('types must be namespaced so they cannot shadow core widgets', () => {
  assert.throws(() => registerScreenWidget({ ...camera, type: 'gauge' }), /namespaced/)
  assert.throws(() => registerScreenWidget({ ...camera, type: 'enterprise.' }), /namespaced/)
  assert.throws(() => registerScreenWidget({ ...camera, surfaces: [] }), /surfaces/)
  assert.throws(() => registerScreenWidget({ ...camera, surfaces: ['twin' as never] }), /surfaces/)
  assert.throws(() => registerScreenWidget({ ...camera, label: '' }), /label/)
  assert.deepEqual(screenWidgets(), [])
})

test('bulk registration reports bad entries and keeps the good ones', () => {
  const errors: string[] = []
  registerScreenWidgets([{ ...camera, type: 'camera' }, scadaOnly], m => errors.push(m))
  assert.equal(errors.length, 1)
  assert.deepEqual(screenWidgets().map(d => d.type), ['acme.dial'])
})

test('an unregistered type resolves to the placeholder', () => {
  registerScreenWidget(scadaOnly)
  assert.deepEqual(resolveScreenWidget('enterprise.camera', 'scada'), {
    available: false, type: 'enterprise.camera', message: UNAVAILABLE_WIDGET_MESSAGE,
  })
  // Registered, but not for this surface.
  assert.equal(resolveScreenWidget('acme.dial', 'dashboard').available, false)
  const ok = resolveScreenWidget('acme.dial', 'scada')
  assert.ok(ok.available && ok.def === scadaOnly)
})

test('sizes and configs have defaults, and configs are not shared', () => {
  assert.deepEqual(dashboardSizeOf(camera), { w: 8, h: 6 })
  assert.deepEqual(dashboardSizeOf(scadaOnly), { w: 6, h: 5 })
  assert.deepEqual(scadaSizeOf(scadaOnly), { width: 320, height: 240 })
  const a = initialConfigOf(camera)
  a.fit = 'contain'
  assert.equal(camera.defaultConfig!.fit, 'cover')
  assert.deepEqual(initialConfigOf(undefined), {})
})

const sensorItem = (i: string, y = 0) => ({ i, x: 0, y, w: 6, h: 4 })

test('dashboard widgets are stored as grid items and read back', () => {
  const layouts = withDashboardWidgets(
    { lg: [sensorItem('1')], md: [sensorItem('1')], sm: [sensorItem('1')] },
    [{ id: 'a', type: 'enterprise.camera', config: { camera_id: 3 } }],
    () => ({ w: 8, h: 6 }),
  )
  assert.deepEqual(layouts.lg[1], { i: 'widget:a', x: 0, y: 4, w: 8, h: 6, widget: { type: 'enterprise.camera', config: { camera_id: 3 } } })
  // Clamped to the narrow breakpoint's columns.
  assert.equal(layouts.sm[1].w, 4)
  assert.equal(layouts.xs, undefined)
  assert.deepEqual(extractDashboardWidgets(layouts), [{ id: 'a', type: 'enterprise.camera', config: { camera_id: 3 } }])
})

test('widgets keep their position, take the new config, and can be removed', () => {
  const base = {
    lg: [sensorItem('1'), { i: 'widget:a', x: 6, y: 0, w: 8, h: 6, widget: { type: 'enterprise.camera', config: { camera_id: 1 } } }],
    md: [sensorItem('1')],
    sm: [],
  }
  const updated = withDashboardWidgets(base, [{ id: 'a', type: 'enterprise.camera', config: { camera_id: 2 } }], () => ({ w: 6, h: 5 }))
  assert.deepEqual(updated.lg[1], { i: 'widget:a', x: 6, y: 0, w: 8, h: 6, widget: { type: 'enterprise.camera', config: { camera_id: 2 } } })
  // Missing from md: added there below the cards.
  assert.equal(updated.md[1].i, widgetItemKey('a'))
  assert.equal(updated.md[1].y, 4)
  const removed = withDashboardWidgets(updated, [], () => ({ w: 6, h: 5 }))
  assert.deepEqual(removed.lg, [sensorItem('1')])
  assert.deepEqual(removed.md, [sensorItem('1')])
})

test('a widget whose type this build lacks is preserved untouched', () => {
  const saved = { lg: [{ i: 'widget:x', x: 2, y: 3, w: 4, h: 4, widget: { type: 'other.thing', config: { k: [1, 2] } } }], md: [], sm: [] }
  const widgets = extractDashboardWidgets(saved)
  assert.deepEqual(widgets, [{ id: 'x', type: 'other.thing', config: { k: [1, 2] } }])
  const again = withDashboardWidgets(saved, widgets, () => ({ w: 1, h: 1 }))
  assert.deepEqual(again.lg, saved.lg)
})

test('items without a readable payload are ignored', () => {
  assert.deepEqual(extractDashboardWidgets({ lg: [{ i: 'widget:z', x: 0, y: 0, w: 1, h: 1 }], md: [], sm: [] }), [])
  assert.deepEqual(extractDashboardWidgets(null), [])
})
