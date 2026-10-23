import { useRef, useState } from 'react'
import { btnStyle } from '../styles'

/** Wraps content so a file dropped on it is passed to `onFiles`; shows an outline while a file is dragged over. */
export function FileDropZone({ onFiles, children, style }: { onFiles: (files: File[]) => void; children: React.ReactNode; style?: React.CSSProperties }) {
  const [over, setOver] = useState(false)
  return (
    <div
      style={{ ...style, outline: over ? '2px dashed var(--green)' : undefined, outlineOffset: -2 }}
      onDragOver={e => { if (e.dataTransfer.types.includes('Files')) { e.preventDefault(); setOver(true) } }}
      onDragLeave={() => setOver(false)}
      onDrop={e => {
        e.preventDefault()
        setOver(false)
        const files = Array.from(e.dataTransfer.files)
        if (files.length > 0) onFiles(files)
      }}
    >
      {children}
    </div>
  )
}

/** A button that opens the file picker; the same `onFiles` as the drop zone. */
export function ImportFileButton({ onFiles, label = 'Import file', ariaLabel }: { onFiles: (files: File[]) => void; label?: string; ariaLabel: string }) {
  const input = useRef<HTMLInputElement>(null)
  return (
    <>
      <input
        ref={input}
        type="file"
        accept=".json,application/json"
        multiple
        hidden
        onChange={e => {
          const files = Array.from(e.target.files ?? [])
          e.target.value = ''
          if (files.length > 0) onFiles(files)
        }}
      />
      <button style={{ ...btnStyle, borderColor: 'var(--muted)', color: 'var(--muted)' }} onClick={() => input.current?.click()} aria-label={ariaLabel}>{label}</button>
    </>
  )
}

/** One line of import result, with a way to dismiss it. */
export function ImportNotice({ lines, onDismiss }: { lines: string[]; onDismiss: () => void }) {
  if (lines.length === 0) return null
  return (
    <div role="status" style={{ marginBottom: 12, padding: '8px 12px', border: '1px solid var(--border)', borderRadius: 8, background: 'var(--card)', fontSize: 12, display: 'flex', gap: 12, alignItems: 'flex-start' }}>
      <div style={{ flex: 1, minWidth: 0 }}>{lines.map((l, i) => <div key={i}>{l}</div>)}</div>
      <button onClick={onDismiss} aria-label="Dismiss" style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer' }}>×</button>
    </div>
  )
}
