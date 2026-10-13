/// A click handler that ignores the second click of a double-click (`detail` above 1). A list
/// drops the card or row a click acted on and the next one moves under the pointer, so the
/// second click would act on that one. A click made with the keyboard has `detail` 0.
export function onSingleClick(action: () => void): (event: { detail: number }) => void {
  return (event) => {
    if (event.detail <= 1) action();
  };
}
