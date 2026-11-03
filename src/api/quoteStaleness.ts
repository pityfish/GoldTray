import { ref, watchEffect } from "vue";

export function useQuoteStaleness(
  fetchedAt: () => string | undefined,
  thresholdSecs: () => number,
) {
  const isStale = ref(false);

  watchEffect((onCleanup) => {
    const timestamp = fetchedAt();
    if (timestamp === undefined) {
      isStale.value = false;
      return;
    }

    const deadline = Date.parse(timestamp) + thresholdSecs() * 1000;
    let timer: number | undefined;
    function update() {
      const delay = deadline - Date.now();
      isStale.value = !Number.isFinite(delay) || delay <= 0;
      if (!isStale.value) {
        timer = window.setTimeout(update, Math.min(delay, 2_147_483_647));
      }
    }

    update();
    onCleanup(() => window.clearTimeout(timer));
  });

  return isStale;
}
