<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";
import { usePlaybackStore } from "./stores/playback";
import { useProfileStore } from "./stores/profile";

const route = useRoute();
const playback = usePlaybackStore();
const profile = useProfileStore();

// On /watch the video is behind the webview, so the page stays transparent.
const transparent = computed(() => route.meta.transparent === true);

onMounted(async () => {
  await Promise.all([playback.subscribe(), profile.refresh()]);
});
</script>

<template>
  <div :class="[$style.shell, transparent ? null : $style.opaque]">
    <!--
      Ambient lighting, only on opaque routes: on /watch any painted pixel here
      would hide the film. The glows live on their own inert layer rather than
      on the background so the cards up top can blur them with backdrop-filter.
    -->
    <div v-if="!transparent" :class="$style.ambient" aria-hidden="true">
      <div :class="$style.field" />
      <div :class="$style.light" />
    </div>
    <RouterView />
  </div>
</template>

<style module>
.shell {
  height: 100%;
  width: 100%;
}

/* `overflow: hidden` keeps the blurred orbs out of the scrollbars. */
.opaque {
  position: relative;
  overflow: hidden;
  background: radial-gradient(120% 100% at 50% 0%, #171a21 0%, #0b0c10 60%, #08090c 100%);
}

.ambient {
  pointer-events: none;
  position: absolute;
  inset: 0;
  overflow: hidden;
}

.field,
.light {
  position: absolute;
}

/* One element, so the colours flow into each other instead of reading as
   separate spots: light from the top, indigo down the sides, violet pooling
   along the bottom edge. */
.field {
  inset: 0;
  background:
    radial-gradient(90% 70% at 50% 0%, var(--c-field-top), transparent 55%),
    radial-gradient(85% 60% at 50% 100%, var(--c-field-bottom), transparent 58%),
    linear-gradient(
      180deg,
      var(--c-field-indigo) 0%,
      transparent 32% 68%,
      var(--c-field-violet) 100%
    );
  animation: fieldBreathe 16s ease-in-out infinite;
}

/* A white spotlight centred on the window, falling from the top. */
.light {
  left: 50%;
  top: -24rem;
  width: 120%;
  height: 40rem;
  transform: translateX(-50%);
  background: radial-gradient(55% 60% at 50% 12%, var(--c-sheen), transparent 62%);
  filter: blur(48px);
  animation: sheenBreathe 9s ease-in-out infinite;
}

@keyframes fieldBreathe {
  0%,
  100% {
    opacity: 0.82;
  }
  50% {
    opacity: 1;
  }
}

@keyframes sheenBreathe {
  0%,
  100% {
    opacity: 0.72;
  }
  50% {
    opacity: 1;
  }
}
</style>
