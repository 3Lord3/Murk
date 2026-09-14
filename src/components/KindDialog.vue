<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useEventListener } from "@vueuse/core";
import type { MediaKind } from "../stores/library";
const emit = defineEmits<{ select: [kind: MediaKind]; cancel: [] }>();
const { t } = useI18n();

const card = ref<HTMLElement | null>(null);
const cancelButton = ref<HTMLButtonElement | null>(null);

// Cancel is what the keyboard lands on first: picking a shelf scans a folder,
// and the harmless option should be the one under the finger.
onMounted(() => cancelButton.value?.focus());

useEventListener(window, "keydown", (event: KeyboardEvent) => {
  if (event.key !== "Escape") return;
  event.preventDefault();
  emit("cancel");
});

// A click on the backdrop dismisses, a click inside the card does not. The
// library closes its "…" menu on any window click, so the event must not
// travel further than this dialog either way.
function onBackdropClick(event: MouseEvent) {
  event.stopPropagation();
  if (!card.value?.contains(event.target as Node)) emit("cancel");
}
</script>

<template>
  <div :class="$style.backdrop" @click="onBackdropClick">
    <div ref="card" :class="$style.card" role="dialog" aria-modal="true" :aria-label="t('library.kind.title')">
      <h2 :class="$style.title">{{ t("library.kind.title") }}</h2>
      <p :class="$style.body">{{ t("library.kind.body") }}</p>

      <div :class="$style.choices">
        <button :class="$style.choice" @click="emit('select', 'series')">
          <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M8.16,3L6.75,4.41L9.34,7H4C2.89,7 2,7.89 2,9V19C2,20.11 2.89,21 4,21H20C21.11,21 22,20.11 22,19V9C22,7.89 21.11,7 20,7H14.66L17.25,4.41L15.84,3L12,6.84L8.16,3M4,9H17V19H4V9M19.5,9A1,1 0 0,1 20.5,10A1,1 0 0,1 19.5,11A1,1 0 0,1 18.5,10A1,1 0 0,1 19.5,9M19.5,12A1,1 0 0,1 20.5,13A1,1 0 0,1 19.5,14A1,1 0 0,1 18.5,13A1,1 0 0,1 19.5,12Z" />
          </svg>
          <span :class="$style.choiceLabel">
            {{ t("library.kind.seriesLabel") }}
          </span>
        </button>

        <button :class="$style.choice" @click="emit('select', 'movie')">
          <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M20.84 2.18L16.91 2.96L19.65 6.5L21.62 6.1L20.84 2.18M13.97 3.54L12 3.93L14.75 7.46L16.71 7.07L13.97 3.54M9.07 4.5L7.1 4.91L9.85 8.44L11.81 8.05L9.07 4.5M4.16 5.5L3.18 5.69A2 2 0 0 0 1.61 8.04L2 10L6.9 9.03L4.16 5.5M2 10V20C2 21.11 2.9 22 4 22H20C21.11 22 22 21.11 22 20V10H2Z" />
          </svg>
          <span :class="$style.choiceLabel">
            {{ t("library.kind.movieLabel") }}
          </span>
        </button>
      </div>

      <button ref="cancelButton" :class="$style.cancel" @click="emit('cancel')">
        {{ t("common.cancel") }}
      </button>
    </div>
  </div>
</template>

<style module>
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 20;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
  background: rgba(0, 0, 0, 0.7);
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
  padding: 1.5rem;
}

.card::after {
  content: "";
  pointer-events: none;
  position: absolute;
  inset: 0;
  background: radial-gradient(140% 80% at 50% 0%, rgba(255, 255, 255, 0.12), transparent 55%);
}

.title {
  font-size: 1.125rem;
  font-weight: 500;
  color: var(--c-text-strong);
}

.body {
  margin-top: 0.5rem;
  font-size: 0.875rem;
  line-height: 1.5;
  color: var(--c-text-muted);
}

.choices {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.75rem;
  margin-top: 1.5rem;
}

.choice {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.625rem;
  border-radius: var(--r-lg);
  background: var(--c-glass);
  box-shadow: inset 0 0 0 1px var(--c-hairline-soft);
  padding: 1.25rem 0.5rem;
  color: var(--c-text);
  transition: background-color var(--t-fast), box-shadow var(--t-fast), transform var(--t-fast);
}

.choice:hover,
.choice:focus-visible {
  background: var(--c-surface-raised);
  box-shadow: inset 0 0 0 1px var(--c-hairline);
  color: var(--c-text-strong);
  transform: translateY(-1px);
}

.icon {
  width: 2.5rem;
  height: 2.5rem;
  fill: var(--c-accent);
}

.choiceLabel {
  font-size: 0.875rem;
  font-weight: 500;
  color: inherit;
}

.cancel {
  margin-top: 0.75rem;
  width: 100%;
  border-radius: var(--r-md);
  background: var(--c-surface-raised);
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  color: var(--c-text);
  transition: background-color var(--t-fast);
}

.cancel:hover,
.cancel:focus-visible {
  background: #232b3d;
}

@media (min-width: 640px) {
  .choice {
    padding-inline: 1rem;
  }
}
</style>