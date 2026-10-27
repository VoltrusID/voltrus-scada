/** One seven-segment line, in the digit's own SVG units. */
export interface LedLine { x1: number; y1: number; x2: number; y2: number }

export const LED_DIGIT_W = 22
export const LED_DIGIT_H = 38

/** The seven segments a, b, c, d, e, f, g of a digit whose left edge is `ox`; each is drawn once. */
export function ledSegmentLines(ox: number): LedLine[] {
  const dw = LED_DIGIT_W
  const dh = LED_DIGIT_H
  return [
    { x1: ox + 3, y1: 1, x2: ox + dw - 3, y2: 1 },
    { x1: ox + dw + 1, y1: 3, x2: ox + dw + 1, y2: dh / 2 - 1 },
    { x1: ox + dw + 1, y1: dh / 2 + 1, x2: ox + dw + 1, y2: dh - 3 },
    { x1: ox + 3, y1: dh / 2, x2: ox + dw - 3, y2: dh / 2 },
    { x1: ox - 1, y1: dh / 2 + 1, x2: ox - 1, y2: dh - 3 },
    { x1: ox - 1, y1: 3, x2: ox - 1, y2: dh / 2 - 1 },
    { x1: ox + 3, y1: dh - 1, x2: ox + dw - 3, y2: dh - 1 },
  ]
}
