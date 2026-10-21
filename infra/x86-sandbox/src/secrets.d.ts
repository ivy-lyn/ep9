// Secrets are not in wrangler.jsonc, so `wrangler types` does not emit them.
interface Env {
	SBX_TOKEN: string;
}
