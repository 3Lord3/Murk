<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";
import { usePlaybackStore } from "../stores/playback";
import { errorMessage } from "../i18n/errors";
import { useProfileStore } from "../stores/profile";

const playback = usePlaybackStore();
const profile = useProfileStore();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();

const confirming = ref(false);
const error = ref<string | null>(null);

const mode = computed(() => profile.current?.peek ?? "disabled");
/** Offered in 5-minute steps, which is also the granularity of the answer. */
const OPTIONS = [10, 15, 20, 30, 45, 60];

async function askFinishWithin(minutes: number) {
  error.value = null;
  try {
    const fits = await invoke<boolean>("can_finish_within", { minutes });
    playback.showPeek(t(fits ? "peek.fits" : "peek.doesNotFit", { minutes }));
    emit("close");
  } catch (e) {
    error.value = String(e);
  }
}

async function revealRemaining() {
  error.value = null;
  try {
    const seconds = await invoke<number>("peek_remaining");
    const minutes = Math.round(seconds / 60);
    playback.showPeek(t("peek.remaining", { minutes }));
    emit("close");
  } catch (e) {
    error.value = String(e);
  }
}

function sentenceCase(text: string): string {
  return text.charAt(0).toLocaleUpperCase() + text.slice(1);
}

async function revealIdentity() {
  error.value = null;
  try {
    const id = await invoke<{ season?: number; number?: number }>("peek_episode_identity");
    const parts = [
      id.season === undefined ? null : t("peek.season", { number: id.season }),
      id.number === undefined ? null : t("peek.episode", { number: id.number }),
    ].filter(Boolean);
    // The catalogue keeps the parts lower case, so whichever ends up first
    // has to start the sentence.
    playback.showPeek(parts.length ? sentenceCase(parts.join(", ")) : t("peek.unknownIdentity"));
    emit("close");
  } catch (e) {
    error.value = String(e);
  }
}
</script>

<template>
  <div :class="$style.backdrop">
    <div :class="$style.card">
      <h2 :class="$style.heading">{{ t("peek.heading") }}</h2>

      <!--
        "Will this be over before I have to go?" is answered with a yes or a
        no, so it reveals neither the position nor the running time.
      -->
      <p :class="$style.lead">{{ t("peek.withinLead") }}</p>
      <div :class="$style.chips">
        <button v-for="m in OPTIONS" :key="m" :class="$style.chip" @click="askFinishWithin(m)">
          {{ t("peek.minutesChip", { minutes: m }) }}
        </button>
      </div>

      <template v-if="mode === 'confirmed'">
        <hr :class="$style.rule" />
        <div v-if="!confirming">
          <p :class="$style.lead">
            {{ t("peek.exactLead") }}
          </p>
          <button :class="$style.reveal" @click="confirming = true">{{ t("peek.exactButton") }}</button>
        </div>
        <div v-else :class="$style.stack">
          <button :class="[$style.chip, $style.wide]" @click="revealRemaining">
            {{ t("peek.remainingButton") }}
          </button>
          <button :class="[$style.chip, $style.wide]" @click="revealIdentity">
            {{ t("peek.identityButton") }}
          </button>
        </div>
      </template>

      <p v-if="error" :class="$style.error">{{ errorMessage(error) }}</p>

      <button :class="$style.close" @click="emit('close')">{{ t("common.close") }}</button>
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
  overflow-y: auto;
  padding: 1rem;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(4px);
}

.card {
  position: relative;
  width: 100%;
  max-width: 26rem;
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
}

/* A pool of light at the top of the card, falling from the window's spotlight. */
.card::after {
  content: "";
  pointer-events: none;
  position: absolute;
  inset: 0;
  background: radial-gradient(140% 80% at 50% 0%, rgba(255, 255, 255, 0.12), transparent 55%);
}

.heading {
  font-size: 1.125rem;
  font-weight: 500;
  color: var(--c-text-strong);
}

.lead {
  margin-top: 0.25rem;
  font-size: 0.875rem;
  color: var(--c-text-muted);
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-top: 0.75rem;
}

.chip {
  border-radius: var(--r-md);
  background: var(--c-veil);
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  color: var(--c-text-strong);
  transition: background-color var(--t-fast);
}

.chip:hover {
  background: var(--c-veil-strong);
}

.wide {
  width: 100%;
}

.stack {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.rule {
  margin: 1.25rem 0;
  border: 0;
  border-top: 1px solid var(--c-hairline);
}

.reveal {
  margin-top: 0.75rem;
  width: 100%;
  border-radius: var(--r-md);
  background: var(--c-warn-bg);
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  color: var(--c-warn-text);
  transition: background-color var(--t-fast);
}

.reveal:hover {
  background: rgba(120, 53, 15, 0.7);
}

.error {
  margin-top: 1rem;
  font-size: 0.875rem;
  color: var(--c-error-text);
}

.close {
  margin-top: 1.5rem;
  width: 100%;
  border-radius: var(--r-md);
  background: var(--c-surface-raised);
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  color: var(--c-text);
  transition: background-color var(--t-fast);
}

.close:hover {
  background: #232b3d;
}

@media (min-width: 640px) {
  .card {
    padding: 1.5rem;
  }
}
</style>
