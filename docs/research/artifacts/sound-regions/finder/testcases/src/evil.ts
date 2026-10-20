export class BadEval { x = 1; }
export function runEval(s: string): unknown { return eval(s); }
