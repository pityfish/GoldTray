import { listen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import { onBeforeUnmount } from "vue";

export function useComponentListener<T>(event: string, handler: EventCallback<T>) {
  let disposed = false;
  let unlisten: UnlistenFn | undefined;

  onBeforeUnmount(() => {
    disposed = true;
    return unlisten?.();
  });

  return async function register() {
    if (disposed) return;
    const stop = await listen<T>(event, (message) => {
      if (!disposed) handler(message);
    });
    if (disposed) {
      await stop();
    } else {
      unlisten = stop;
    }
  };
}
