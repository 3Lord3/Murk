import { createRouter, createWebHashHistory } from "vue-router";
import LibraryView from "./views/LibraryView.vue";
import WatchView from "./views/WatchView.vue";
import SettingsView from "./views/SettingsView.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "library", component: LibraryView },
    // The only route that must not paint a background: the video is behind it.
    { path: "/watch", name: "watch", component: WatchView, meta: { transparent: true } },
    { path: "/settings", name: "settings", component: SettingsView },
  ],
});
