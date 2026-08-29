// Compact squarified treemap layout (Bruls et al.): returns absolute rects
// for weighted items inside a container. Pure math, no DOM.

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Layout weights (already sorted descending) into rects filling width×height. */
export function squarify(weights: number[], width: number, height: number): Rect[] {
  const total = weights.reduce((a, b) => a + b, 0);
  const rects: Rect[] = [];
  if (total <= 0 || weights.length === 0) return rects;

  // Scale weights to areas in pixel² space.
  const scaled = weights.map((w) => (w / total) * width * height);
  let x = 0;
  let y = 0;
  let remW = width;
  let remH = height;
  let row: number[] = [];
  let i = 0;

  /** Worst aspect ratio of the current row laid along `side`. */
  const worst = (candidate: number[], side: number): number => {
    const sum = candidate.reduce((a, b) => a + b, 0);
    const max = Math.max(...candidate);
    const min = Math.min(...candidate);
    const s2 = sum * sum;
    return Math.max((side * side * max) / s2, s2 / (side * side * min));
  };

  /** Flush the current row into rects along the shorter side. */
  const layoutRow = () => {
    const sum = row.reduce((a, b) => a + b, 0);
    const horizontal = remW >= remH;
    const thickness = sum / (horizontal ? remH : remW);
    let offset = 0;
    for (const area of row) {
      const length = area / thickness;
      if (horizontal) {
        rects.push({ x, y: y + offset, w: thickness, h: length });
      } else {
        rects.push({ x: x + offset, y, w: length, h: thickness });
      }
      offset += length;
    }
    if (horizontal) {
      x += thickness;
      remW -= thickness;
    } else {
      y += thickness;
      remH -= thickness;
    }
    row = [];
  };

  while (i < scaled.length) {
    const side = Math.min(remW, remH);
    const next = scaled[i];
    if (row.length === 0 || worst([...row, next], side) <= worst(row, side)) {
      row.push(next);
      i += 1;
    } else {
      layoutRow();
    }
  }
  if (row.length > 0) layoutRow();
  return rects;
}
