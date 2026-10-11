/** Immutable interval snapshot. Build on document changes, never on transport ticks. */
export class IntervalIndex<T> {
  private rows: Array<{ value: T; start: number; end: number }>;
  private maximum: number[] = [];
  constructor(values: readonly T[], start: (value: T) => number, end: (value: T) => number) {
    this.rows = values.map(value => ({ value, start: start(value), end: end(value) })).sort((a, b) => a.start - b.start);
    const build = (lo: number, hi: number, node: number): number => {
      if (lo >= hi) return -Infinity;
      const mid = (lo + hi) >>> 1;
      return this.maximum[node] = Math.max(this.rows[mid]!.end, build(lo, mid, node * 2 + 1), build(mid + 1, hi, node * 2 + 2));
    };
    build(0, this.rows.length, 0);
  }
  intersect(from: number, to: number): T[] {
    const result: T[] = [];
    const visit = (lo: number, hi: number, node: number) => {
      if (lo >= hi || this.maximum[node]! <= from || this.rows[lo]!.start >= to) return;
      const mid = (lo + hi) >>> 1, row = this.rows[mid]!;
      visit(lo, mid, node * 2 + 1);
      if (row.start < to && row.end > from) result.push(row.value);
      visit(mid + 1, hi, node * 2 + 2);
    };
    if (to > from) visit(0, this.rows.length, 0);
    return result;
  }
  at(time: number): T[] { return this.intersect(time, time + 1); }
  upcoming(time: number, until: number, limit: number): T[] {
    let lo = 0, hi = this.rows.length;
    while (lo < hi) { const mid = (lo + hi) >>> 1; if (this.rows[mid]!.start <= time) lo = mid + 1; else hi = mid; }
    const result: T[] = [];
    for (let i = lo; i < this.rows.length && result.length < limit && this.rows[i]!.start <= until; i++) result.push(this.rows[i]!.value);
    return result;
  }
}

export function nextBoundary(boundaries: readonly number[], time: number, end: number): number {
  let lo = 0, hi = boundaries.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (boundaries[mid]! <= time) lo = mid + 1; else hi = mid; }
  return boundaries[lo] ?? end;
}
