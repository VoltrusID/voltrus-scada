// SCADA-17 B11: the shift handover list showed `by #<id>` because the server sent only the id.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { authorLabel } from '../src/authorLabel.ts'

test('a handover is shown by its author\'s username', () => {
  assert.equal(authorLabel('maria', 3), 'maria')
})

test('an author that no longer exists falls back to the id', () => {
  assert.equal(authorLabel(null, 3), '#3')
  assert.equal(authorLabel(undefined, 3), '#3')
  assert.equal(authorLabel('', 3), '#3')
})
