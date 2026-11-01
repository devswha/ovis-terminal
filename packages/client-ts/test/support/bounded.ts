// Hang guard for event-driven waits: it bounds a wait, it never decides an outcome.
export const HANG_GUARD_MS = 20_000;

export function within<T>(promise: Promise<T>, what: string, ms = HANG_GUARD_MS): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const guard = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`hang guard: ${what} did not happen within ${ms} ms`)), ms);
  });
  return Promise.race([promise, guard]).finally(() => clearTimeout(timer));
}

/** Resolves with the first value `register` delivers; register before triggering the action. */
export function once<T>(register: (deliver: (value: T) => void) => () => void): Promise<T> {
  return new Promise((resolve) => {
    const stop = register((value) => {
      stop();
      resolve(value);
    });
  });
}

export async function rejection(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("expected a rejection, got a result");
}
