// A clean file. The string has characters of 2, 3 and 4 UTF-8 bytes.
export class Point {
  x: number;
  y: number;
  constructor(x: number, y: number) {
    this.x = x;
    this.y = y;
  }
}

export const label: string = "größe € \u{1f600}: größe € 😀";

export function norm(p: Point): number {
  return Math.sqrt(p.x * p.x + p.y * p.y);
}
