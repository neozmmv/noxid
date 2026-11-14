// Which view the shell shows. Shared state rather than a router: there are
// four views, no URLs, and no history to go back through.

export type View = "home" | "friends" | "servers" | "settings";

export const nav = $state<{ view: View }>({ view: "home" });

export function go(view: View): void {
  nav.view = view;
}
