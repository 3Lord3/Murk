<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";

// The whole payload of the end-of-file event: something follows, or nothing
// does. It never says what.
const props = defineProps<{ hasNext: boolean }>();
const emit = defineEmits<{ done: [] }>();
const router = useRouter();
const { t } = useI18n();

const COUNTDOWN_SECONDS = 8;
const remaining = ref(COUNTDOWN_SECONDS);
let timer: number | undefined;

async function go() {
  window.clearInterval(timer);
  emit("done");
  await invoke("play_next");
}

async function cancel() {
  window.clearInterval(timer);
  await invoke("cancel_next");
  emit("done");
  await invoke("stop");
  router.push("/");
}

/** Close the card and leave the player where it is: parked on the last frame,
    so a seek back into the final minutes still works. */
function stay() {
  window.clearInterval(timer);
  emit("done");
}

onMounted(() => {
  // Nothing follows a film or a finale, so there is nothing to count down to.
  // The card still appears, as the only signal that the file ended, but it
  // waits for the user instead of acting on its own.
  if (!props.hasNext) return;
  timer = window.setInterval(() => {
    remaining.value -= 1;
    if (remaining.value <= 0) void go();
  }, 1000);
});

onUnmounted(() => window.clearInterval(timer));
</script>

<template>
  <!--
    The end of an episode is where a normal player is loudest ("Next: S03E10,
    The Rains of Castamere, 58 min"). This prompt says only that something
    follows, and offers a way out of it.
  -->
  <div :class="$style.backdrop">
    <div :class="$style.card">
      <p :class="$style.lead">{{ hasNext ? t("advance.lead") : t("advance.endLead") }}</p>
      <p v-if="hasNext" :class="$style.countdown">
        {{ t("advance.countdown", { seconds: remaining }) }}
      </p>

      <div :class="$style.actions">
        <template v-if="hasNext">
          <button :class="[$style.button, $style.primary]" @click="go">{{ t("advance.watch") }}</button>
          <button :class="[$style.button, $style.secondary]" @click="cancel">{{ t("advance.stop") }}</button>
        </template>
        <template v-else>
          <button :class="[$style.button, $style.primary]" @click="cancel">{{ t("advance.library") }}</button>
          <button :class="[$style.button, $style.secondary]" @click="stay">{{ t("advance.stay") }}</button>
        </template>
      </div>
    </div>
  </div>
</template>

<style module>
.backdrop {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
  background: rgba(0, 0, 0, 0.7);
}

.card {
  position: relative;
  width: 100%;
  max-width: 24rem;
  overflow: hidden;
  border-radius: var(--r-xl);
  background: linear-gradient(
    165deg,
    rgba(255, 255, 255, 0.08) 0%,
    rgba(26, 32, 48, 0.85) 45%,
    rgba(15, 19, 27, 0.92) 100%
  );
  box-shadow:
    inset 0 0 0 1px var(--c-hairline),
    inset 0 1px 0 rgba(255, 255, 255, 0.12),
    0 24px 56px rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(20px);
  padding: 1rem;
  text-align: center;
}

/* A pool of light at the top of the card, falling from the window's spotlight. */
.card::after {
  content: "";
  pointer-events: none;
  position: absolute;
  inset: 0;
  background: radial-gradient(140% 80% at 50% 0%, rgba(255, 255, 255, 0.12), transparent 55%);
}

.lead {
  font-size: 1rem;
  color: var(--c-text);
}

.countdown {
  margin-top: 0.25rem;
  font-size: 0.875rem;
  color: var(--c-text-faint);
}

.actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 1.25rem;
}

.button {
  flex: 1;
  border-radius: var(--r-md);
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  transition: background-color var(--t-fast);
}

.primary {
  background: var(--c-accent);
  color: var(--c-accent-text);
  font-weight: 500;
}

.primary:hover {
  background: #fff;
}

.secondary {
  background: var(--c-surface-raised);
  color: var(--c-text);
}

.secondary:hover {
  background: #232b3d;
}

@media (min-width: 640px) {
  .card {
    padding: 1.5rem;
  }
}
</style>
