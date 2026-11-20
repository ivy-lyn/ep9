// A file with a type error (TS2322): its hints are dropped (H37 S3).
import { Point } from "./ok";

export const n: number = "not a number";
export const p: Point = new Point(1, 2);
