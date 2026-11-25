import assert from 'node:assert/strict'
import { test } from 'node:test'
import { axisLabel, distinctColors, overlayScales, sideLabel } from '../src/trend/overlayAxes.ts'

test('each unit gets its own axis', () => {
  // Raw turbidity (NTU) with the clearwell level (%) and chlorine residual (mg/L).
  assert.deepEqual(overlayScales('NTU', ['%', 'mg/L']), [
    { id: 'left', side: 'left' },
    { id: 'unit:mg/L', side: 'own' },
  ])
})

test('the primary unit shares the right axis, and equal units share a scale', () => {
  assert.deepEqual(overlayScales('NTU', ['NTU', '%', ' % ', 'mg/L', 'mg/L']), [
    { id: 'right', side: 'right' },
    { id: 'left', side: 'left' },
    { id: 'left', side: 'left' },
    { id: 'unit:mg/L', side: 'own' },
    { id: 'unit:mg/L', side: 'own' },
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
