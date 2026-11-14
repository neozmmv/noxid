// One short message at the bottom of the window, from anywhere: notices from
// the backend and outcomes of what the user just did.

export const toast = $state<{ message: string | null }>({ message: null });

let timer: ReturnType<typeof setTimeout> | undefined;

export function showToast(message: string, ms = 6000): void {
  toast.message = message;
  clearTimeout(timer);
  timer = setTimeout(() => (toast.message = null), ms);
}
