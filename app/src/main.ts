import "@fontsource-variable/familjen-grotesk";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

document.addEventListener("contextmenu", (event) => {
  const target = event.target as HTMLElement | null;
  const editable = target?.closest("input, textarea, [contenteditable='true']");
  const selected = (window.getSelection()?.toString() ?? "").length > 0;
  if (!editable && !selected) event.preventDefault();
});

const target = document.getElementById("app");
if (!target) throw new Error("missing #app");
mount(App, { target });
