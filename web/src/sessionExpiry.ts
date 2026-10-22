const maxTimerDelay = 2_147_483_647;

export function expireSessionAt(
  expiresAt: number,
  onExpire: () => void,
): () => void {
  const deadline = expiresAt * 1000;
  let timer: number | undefined;
  const schedule = () => {
    timer = window.setTimeout(
      () => {
        if (Date.now() >= deadline) onExpire();
        else schedule();
      },
      Math.min(Math.max(0, deadline - Date.now()), maxTimerDelay),
    );
  };
  schedule();
  return () => {
    window.clearTimeout(timer);
  };
}
