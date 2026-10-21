// An infinite range has a null offset and limit. Where the old arithmetic read a null as 0
// (JS coerces it that way), the code below says `?? 0`.
export class QueryRange {
  static infinite() {
    return new QueryRange({ limit: null, offset: null });
  }

  static rangeWithUnion(a: QueryRange, b: QueryRange) {
    if (a.isInfinite() || b.isInfinite()) {
      return QueryRange.infinite();
    }
    if (!a.isContiguousWith(b)) {
      throw new Error('You cannot union ranges which do not touch or intersect.');
    }

    return new QueryRange({
      start: Math.min(a.start ?? 0, b.start ?? 0),
      end: Math.max(a.end, b.end),
    });
  }

  static rangesBySubtracting(a: QueryRange, b: QueryRange) {
    if (!b) {
      return [];
    }

    if (a.isInfinite() || b.isInfinite()) {
      throw new Error('You cannot subtract infinite ranges.');
    }

    const uncovered: QueryRange[] = [];
    const aStart = a.start ?? 0;
    const bStart = b.start ?? 0;
    if (bStart > aStart) {
      uncovered.push(new QueryRange({ start: a.start, end: Math.min(a.end, bStart) }));
    }
    if (b.end < a.end) {
      uncovered.push(new QueryRange({ start: Math.max(aStart, b.end), end: a.end }));
    }
    return uncovered;
  }

  get start() {
    return this.offset;
  }

  get end() {
    return (this.offset ?? 0) + (this.limit ?? 0);
  }

  offset: number | null;
  limit: number | null;

  constructor({
    limit,
    offset,
    start,
    end,
  }: {
    limit?: number | null;
    offset?: number | null;
    start?: number | null;
    end?: number;
  } = {}) {
    let rangeLimit = limit;
    let rangeOffset = offset;

    if (start !== undefined && offset === undefined) {
      rangeOffset = start;
    }
    if (end !== undefined && limit === undefined) {
      rangeLimit = end - (rangeOffset ?? 0);
    }

    if (rangeLimit === undefined) {
      throw new Error('You must specify a limit');
    }
    if (rangeOffset === undefined) {
      throw new Error('You must specify an offset');
    }
    this.limit = rangeLimit;
    this.offset = rangeOffset;
  }

  clone() {
    const { limit, offset } = this;
    return new QueryRange({ limit, offset });
  }

  isInfinite() {
    return this.limit === null && this.offset === null;
  }

  isEqual(b: QueryRange) {
    return this.start === b.start && this.end === b.end;
  }

  // Returns true if joining the two ranges would not result in empty space.
  // ie: they intersect or touch
  isContiguousWith(b: QueryRange) {
    if (this.isInfinite() || b.isInfinite()) {
      return true;
    }
    const start = this.start ?? 0;
    const bStart = b.start ?? 0;
    return (start <= bStart && bStart <= this.end) || (start <= b.end && b.end <= this.end);
  }

  toString() {
    return `QueryRange{${this.start} - ${this.end}}`;
  }
}
