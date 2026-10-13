// A stand-in for the Rust side in component tests: commands answer from handlers the test
// installs, and the test emits events. Wired in by test/setup.ts.
type Handler = (args: Record<string, unknown>) => unknown;
type Listener = (event: { payload: unknown }) => void;

/// An answer the test gives or refuses later: a command still on its way, or one that read its
/// rows before something changed.
export function later<T>() {
  let give!: (value: T) => void;
  let fail!: (error: unknown) => void;
  const answer = new Promise<T>((resolve, reject) => {
    give = resolve;
    fail = reject;
  });
  return { answer, give, fail };
}

class FakeBackend {
  private handlers = new Map<string, Handler>();
  private listeners = new Map<string, Set<Listener>>();
  /// While set, `listen` waits for it before it attaches a listener.
  private attaching: Promise<void> | null = null;
  /// Every command invoked, in order, with its arguments.
  calls: [string, Record<string, unknown>][] = [];

  /// Answers `cmd` with what `handler` returns; a thrown value rejects the command.
  on(cmd: string, handler: Handler): void {
    this.handlers.set(cmd, handler);
  }

  /// The arguments of every call to `cmd`.
  callsTo(cmd: string): Record<string, unknown>[] {
    return this.calls.filter(([name]) => name === cmd).map(([, args]) => args);
  }

  emit(event: string, payload: unknown): void {
    for (const listener of this.listeners.get(event) ?? []) listener({ payload });
  }

  /// Makes `listen` as slow as the real bridge can be: from now on a listener hears nothing
  /// until the returned function is called, and what is emitted before that is lost.
  holdListeners(): () => void {
    const held = later<void>();
    this.attaching = held.answer;
    return () => held.give();
  }

  reset(): void {
    this.handlers.clear();
    this.listeners.clear();
    this.attaching = null;
    this.calls = [];
  }

  async invoke(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
    this.calls.push([cmd, args]);
    const handler = this.handlers.get(cmd);
    if (!handler) throw { kind: "internal", message: `the test has no answer for ${cmd}` };
    return handler(args);
  }

  async listen(event: string, listener: Listener): Promise<() => void> {
    if (this.attaching) await this.attaching;
    const set = this.listeners.get(event) ?? new Set();
    this.listeners.set(event, set);
    set.add(listener);
    return () => set.delete(listener);
  }
}

export const backend = new FakeBackend();
