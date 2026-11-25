import assert from 'node:assert/strict'
import { test } from 'node:test'
import { axisLabel, distinctColors, overlayScales, sideLabel, unitKey } from '../src/trend/overlayAxes.ts'

test('each unit gets its own axis', () => {
  // Raw turbidity (NTU) with the clearwell level (%) and chlorine residual (mg/L).
  assert.deepEqual(overlayScales('NTU', ['%', 'mg/L']), [
    { id: 'left', side: 'left' },
    { id: 'unit:mg/l', side: 'own' },
  ])
})

test('the primary unit shares the right axis, and equal units share a scale', () => {
  assert.deepEqual(overlayScales('NTU', ['NTU', '%', ' % ', 'mg/L', 'mg/L']), [
    { id: 'right', side: 'right' },
    { id: 'left', side: 'left' },
    { id: 'left', side: 'left' },
    { id: 'unit:mg/l', side: 'own' },
    { id: 'unit:mg/l', side: 'own' },
  ])
  assert.deepEqual(overlayScales('', ['']), [{ id: 'right', side: 'right' }])
  assert.deepEqual(overlayScales('bar', []), [])
})

test('overlays sharing the primary colour get trend pen colours', () => {
  const pens = ['#0ea5e9', '#f97316', '#a855f7']
  assert.deepEqual(distinctColors('#94a3b8', ['#94A3B8', '#94a3b8', '#16a34a'], pens), ['#0ea5e9', '#f97316', '#16a34a'])
  assert.deepEqual(distinctColors('#0ea5e9', ['#0ea5e9'], pens), ['#f97316'])
  assert.deepEqual(distinctColors('#111111', [], pens), [])
})

test('axis labels carry their own unit', () => {
  assert.equal(axisLabel(2.6022, 'mg/L'), '2.60 mg/L')
  assert.equal(axisLabel(1, ''), '1.00')
  assert.equal(sideLabel('own'), 'own scale')
})

test('units compare without case, spacing or superscripts', () => {
  assert.equal(unitKey(' m³/h '), 'm3/h')
  assert.deepEqual(overlayScales('m3/h', ['M³/h', 'Bar', 'bar']), [
    { id: 'right', side: 'right' },
    { id: 'left', side: 'left' },
    { id: 'left', side: 'left' },
  ])
})

test('colours compare in any spelling', () => {
  const pens = ['#0ea5e9', '#f97316']
  assert.deepEqual(distinctColors('#FFF', [' #ffffff', '#16a34a'], pens), ['#0ea5e9', '#16a34a'])
  assert.deepEqual(distinctColors('#0ea5e9', ['#0EA5E9'], ['#0ea5e9', '#F97316']), ['#F97316'])
})

test('axis labels follow the tag decimals', () => {
  assert.equal(axisLabel(1166.81, 'm3/h', 0), '1167 m3/h')
  assert.equal(axisLabel(2.6022, 'mg/L', 3), '2.602 mg/L')
  assert.equal(axisLabel(-0.001, 'bar', 2), '0.00 bar')
  assert.equal(axisLabel(2.6, '', 40), '2.600000')
})
