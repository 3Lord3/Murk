<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { useIdle, useEventListener } from "@vueuse/core";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { usePlaybackStore } from "../stores/playback";
import { useProfileStore } from "../stores/profile";
import ControlsOverlay from "../components/ControlsOverlay.vue";
import AdvancePrompt from "../components/AdvancePrompt.vue";
import PeekPanel from "../components/PeekPanel.vue";

const playback = usePlaybackStore();
const profile = useProfileStore();
const router = useRouter();
const { t } = useI18n();

// The overlay appears on any input and withdraws after a couple of seconds of
// stillness, so the default state of this screen is the film alone.
const { idle } = useIdle(2500);
// Paused counts as "wants controls": someone who paused is looking for a
// button, not for an unobstructed frame.
const controlsVisible = computed(() => !idle.value || playback.view.paused);

const showAdvance = ref(false);
// What the last end-of-file event said: whether anything follows this file.
const advanceHasNext = ref(false);
const showPeek = ref(false);
let unlistenAdvance: UnlistenFn | undefined;

const SEEK_STEPS = { plain: 10, shift: 60, ctrl: 300 };
const toast = ref<string | null>(null);
let toastTimer: number | undefined;

function flash(message: string) {
  toast.value = message;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (toast.value = null), 1200);
}

async function seek(direction: -1 | 1, event: KeyboardEvent) {
  const step = event.ctrlKey ? SEEK_STEPS.ctrl : event.shiftKey ? SEEK_STEPS.shift : SEEK_STEPS.plain;
  const delta = direction * step;
  try {
    await invoke("seek_relative", { deltaSec: delta });
  } catch {
    // A seek mpv would not take (usually forward from a file that already
    // ended) moves nothing, so it reports nothing.
    return;
  }
  // The *delta*, never the destination: "−1 min" says nothing about where in
  // the episode we are, "42:10" would say everything.
  const label =
    step >= 60
      ? t("watch.seekMinutes", { count: step / 60 })
      : t("watch.seekSeconds", { count: step });
  flash(`${delta < 0 ? "−" : "+"}${label}`);
}

async function onKey(event: KeyboardEvent) {
  if (event.target instanceof HTMLInputElement) return;
  // While the advance prompt is up, a key that seeks or pauses would act on
  // nothing and leave the countdown running. Its buttons are the way through.
  if (showAdvance.value) return;
  switch (event.key) {
    case " ":
    case "k":
      event.preventDefault();
      await invoke("play_pause");
      break;
    case "ArrowLeft":
      event.preventDefault();
      await seek(-1, event);
      break;
    case "ArrowRight":
      event.preventDefault();
      await seek(1, event);
      break;
    case "ArrowUp":
      event.preventDefault();
      await invoke("set_volume", { volume: Math.min(130, playback.view.volume + 5) });
      break;
    case "ArrowDown":
      event.preventDefault();
      await invoke("set_volume", { volume: Math.max(0, playback.view.volume - 5) });
      break;
    case "f":
      await invoke("toggle_fullscreen");
      break;
    case "?":
      showPeek.value = profile.current !== null && profile.current.peek !== "disabled";
      break;
    case "Escape":
      await leave();
      break;
  }
}

async function leave() {
  await invoke("stop");
  await invoke("set_fullscreen", { full: false });
  playback.clearPeek();
  router.push("/");
}

useEventListener(window, "keydown", onKey);

onMounted(async () => {
  unlistenAdvance = await listen<{ hasNext: boolean }>("murk://advance", (event) => {
    // Full darkness hides the seams between files: the card would announce
    // every boundary, and its absence on a finale would announce that too.
    // Episodes simply follow one another, and the last one ends in silence.
    if (profile.current?.id === "full_darkness") {
      if (event.payload.hasNext) void invoke("play_next");
      return;
    }
    // Elsewhere the card comes up whether or not something follows: a film or
    // a finale would otherwise end on a frozen frame with no sign it was over.
    advanceHasNext.value = event.payload.hasNext;
    showAdvance.value = true;
  });
});

onUnmounted(() => {
  unlistenAdvance?.();
  window.clearTimeout(toastTimer);
});
</script>

<template>
  <!--
    Nothing in this subtree may paint an opaque background: the film is a GL
    surface behind the webview, not an element on the page. Only the controls
    colour pixels, and they do it with translucent panels.
  -->
  <div :class="[$style.stage, controlsVisible ? $style.cursorDefault : $style.cursorNone]">
    <!-- Invisible but usable, since the window is undecorated. -->
    <div data-tauri-drag-region :class="$style.dragStrip" />

    <!--
      Transition class names have to be handed over explicitly: under CSS
      modules `.fade-enter-active` would be hashed and the plain `name="fade"`
      convention could never find it.
    -->
    <Transition
      :enter-active-class="$style.fadeActive"
      :leave-active-class="$style.fadeActive"
      :enter-from-class="$style.fadeFrom"
      :leave-to-class="$style.fadeFrom"
    >
      <div v-if="toast" :class="$style.toast">{{ toast }}</div>
    </Transition>

    <Transition
      :enter-active-class="$style.fadeActive"
      :leave-active-class="$style.fadeActive"
      :enter-from-class="$style.fadeFrom"
      :leave-to-class="$style.fadeFrom"
    >
      <ControlsOverlay v-if="controlsVisible" @leave="leave" @peek="showPeek = true" />
    </Transition>

    <PeekPanel v-if="showPeek" @close="showPeek = false" />
    <AdvancePrompt v-if="showAdvance" :has-next="advanceHasNext" @done="showAdvance = false" />
  </div>
</template>

<style module>
.stage {
  position: relative;
  height: 100%;
  width: 100%;
}

.cursorDefault {
  cursor: default;
}

.cursorNone {
  cursor: none;
}

/* WebKitGTK ignores `-webkit-app-region`, so the drag comes from
   `data-tauri-drag-region`; this only stops the pointer from selecting or
   native-dragging content while a drag starts. */
.dragStrip {
  position: absolute;
  inset-inline: 0;
  top: 0;
  height: 2.5rem;
  -webkit-user-select: none;
  user-select: none;
  -webkit-user-drag: none;
}

.toast {
  pointer-events: none;
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  border-radius: var(--r-xl);
  background: var(--c-scrim);
  padding: 0.75rem 1.5rem;
  font-size: 1.875rem;
  font-weight: 300;
  font-variant-numeric: tabular-nums;
  color: #fff;
  backdrop-filter: blur(4px);
}

.fadeActive {
  transition: opacity 180ms ease;
}

.fadeFrom {
  opacity: 0;
}
</style>
