// Deterministic node placement for the canvas. Pure math — no React Flow
// imports — so the grid geometry is unit-testable on its own.
//
// Layout model: row-major grid. The column count follows the square root of
// the node count, biased slightly wide (cards are wider than tall, so a
// 16:10-ish canvas fills more naturally than a strict square), and is capped
// so a huge fleet wraps instead of producing one endless row.

export interface XY {
  x: number;
  y: number;
}

/** Card geometry per node type (px, at zoom 1). */
export const ACCOUNT_NODE = { width: 264, height: 152, gapX: 40, gapY: 36 } as const;
export const REPO_NODE = { width: 224, height: 116, gapX: 32, gapY: 28 } as const;
export const DETAIL_NODE = { width: 460 } as const;

/** Column count for `count` nodes: ceil(sqrt(count * 1.4)), min 1, max 5. */
export function columnsFor(count: number): number {
  if (count <= 0) return 1;
  return Math.min(5, Math.max(1, Math.ceil(Math.sqrt(count * 1.4))));
}

/** Row-major grid positions for `count` nodes of the given geometry. */
export function gridPositions(
  count: number,
  geo: { width: number; height: number; gapX: number; gapY: number },
  cols = columnsFor(count)
): XY[] {
  const positions: XY[] = [];
  for (let i = 0; i < count; i++) {
    const col = i % cols;
    const row = Math.floor(i / cols);
    positions.push({
      x: col * (geo.width + geo.gapX),
      y: row * (geo.height + geo.gapY),
    });
  }
  return positions;
}
