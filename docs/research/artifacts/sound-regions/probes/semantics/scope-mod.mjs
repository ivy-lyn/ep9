const secret = { v: 1 };            // not exported, never escapes
export const pub = { v: 1 };        // exported
export const readSecret = () => typeof secret.v;
