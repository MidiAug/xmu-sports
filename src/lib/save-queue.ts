export type SaveState = "saved" | "pending" | "saving" | "error";
/** Serialize writes across mounts; coalesce edits while a write is in flight. */
export function createSaveQueue<T>(write: (value: T) => Promise<void>, delay = 350) {
  let pending: { value: T } | null = null;
  let active: Promise<void> | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let state: SaveState = "saved";
  let error: unknown = null;
  const listeners = new Set<(state: SaveState, error: unknown) => void>();
  const publish = (next: SaveState, cause: unknown = null) => {
    state = next; error = cause;
    listeners.forEach((listener) => listener(state, error));
  };
  async function flush(): Promise<void> {
    clearTimeout(timer);
    if (active) { await active; if (pending) await flush(); return; }
    const item = pending;
    if (!item) return;
    pending = null;
    publish("saving");
    active = (async () => {
      try {
        await write(item.value);
        publish(pending ? "pending" : "saved");
      } catch (cause) {
        if (!pending) pending = item;
        publish("error", cause);
        throw cause;
      } finally { active = null; }
    })();
    await active;
    if (pending) await flush();
  }
  return {
    enqueue(value: T) {
      pending = { value };
      publish("pending");
      clearTimeout(timer);
      timer = setTimeout(() => { void flush().catch(() => {}); }, delay);
    },
    flush,
    discard() {
      if (active) throw new Error("仍有配置正在保存，请稍后重试");
      clearTimeout(timer); pending = null; publish("saved");
    },
    subscribe(listener: (state: SaveState, error: unknown) => void) {
      listeners.add(listener); listener(state, error);
      return () => { listeners.delete(listener); };
    },
  };
}
