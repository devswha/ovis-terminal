// Failures of `bun run build` and `bun run pack`, each with a stable kind and recovery hint.

const HINTS = {
  stale_generated: "run `bun run generate` so src/generated matches schema/ovis-terminal.openrpc.json",
  bundle_failed: "fix the reported module error in src/ and build again",
  declarations_failed: "fix the reported type error in src/ and build again",
  not_built: "run `bun run build` before `bun run pack`",
  pack_failed: "inspect the bun pm pack output and pack again",
} as const;

export type PackageErrorKind = keyof typeof HINTS;

export class PackageError extends Error {
  readonly kind: PackageErrorKind;
  readonly hint: string;

  constructor(kind: PackageErrorKind, message: string) {
    super(message);
    this.name = "PackageError";
    this.kind = kind;
    this.hint = HINTS[kind];
  }
}

/** Runs `main`; a PackageError becomes one JSON line on stderr and exit status 1. */
export async function exitWith(main: () => Promise<unknown>): Promise<never> {
  try {
    process.stdout.write(`${JSON.stringify(await main())}\n`);
    process.exit(0);
  } catch (error) {
    if (!(error instanceof PackageError)) throw error;
    process.stderr.write(`${JSON.stringify({ kind: error.kind, hint: error.hint, message: error.message })}\n`);
    process.exit(1);
  }
}
