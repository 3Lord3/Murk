<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";

const appWindow = getCurrentWindow();
const { t } = useI18n();
</script>

<template>
  <!--
    The window has no decorations, so this strip is both the drag handle and
    the only place a title could appear. It says "Murk" and nothing else: a
    title bar naming the episode would announce it in every screenshot and
    every window switcher, outside any hiding profile.
  -->
  <header data-tauri-drag-region :class="$style.bar">
    <span data-tauri-drag-region :class="$style.title">Murk</span>
    <div :class="$style.buttons">
      <button :class="$style.button" :title="t('titlebar.minimise')" @click="appWindow.minimize()">–</button>
      <button
        :class="[$style.button, $style.close]"
        :title="t('titlebar.close')"
        @click="appWindow.close()"
      >
        ✕
      </button>
    </div>
  </header>
</template>

<style module>
/* WebKitGTK ignores `-webkit-app-region`, so the actual dragging comes from
   `data-tauri-drag-region`; these rules only stop the pointer from selecting or
   native-dragging content while a drag starts. */
.bar,
.bar * {
  -webkit-user-select: none;
  user-select: none;
  -webkit-user-drag: none;
}

.bar {
  position: relative;
  z-index: 1;
  display: flex;
  height: 2.75rem;
  align-items: center;
  justify-content: space-between;
  padding: 0 1rem;
}

.title {
  font-size: 0.875rem;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  background: linear-gradient(90deg, #eef1f7, #9fb0c8);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.buttons {
  display: flex;
  gap: 0.25rem;
}

.button {
  height: 1.75rem;
  width: 2rem;
  border-radius: var(--r-sm);
  color: var(--c-text-muted);
  transition: background-color var(--t-fast), color var(--t-fast);
}

.button:hover {
  background: var(--c-surface-raised);
  color: var(--c-text-strong);
}

.close:hover {
  background: rgba(127, 29, 29, 0.7);
  color: var(--c-text-strong);
}
</style>
