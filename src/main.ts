import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "./router";
import { i18n, initLocale } from "./i18n";
import "./styles.css";

// Браузерное контекстное меню (back/forward/reload/inspect) в приложении
// неуместно. Поля ввода оставляем как есть.
window.addEventListener("contextmenu", (e) => {
  const el = e.target as HTMLElement | null;
  if (el?.closest("input, textarea, [contenteditable='true']")) return;
  e.preventDefault();
});

// Settled before the first frame, so nobody sees a flash of English.
initLocale().finally(() => {
  createApp(App).use(createPinia()).use(router).use(i18n).mount("#app");
});
