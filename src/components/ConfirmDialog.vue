<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useEventListener } from "@vueuse/core";

const props = withDefaults(
  defineProps<{ title: string; body: string; confirmLabel: string; danger?: boolean }>(),
  { danger: false },
);
const emit = defineEmits<{ confirm: []; cancel: [] }>();
const { t } = useI18n();

const card = ref<HTMLElement | null>(null);
const cancelButton = ref<HTMLButtonElement | null>(null);

// The destructive action is never what the keyboard lands on: a dialog that
// answers Enter with "yes, forget everything" is a trap for someone who was
// still reading it.
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
    <div
      ref="card"
      :class="$style.card"
      role="dialog"
      aria-modal="true"
      :aria-label="props.title"
    >
      <h2 :class="$style.title">{{ props.title }}</h2>
      <p :class="$style.body">{{ props.body }}</p>

      <div :class="$style.actions">
        <button ref="cancelButton" :class="[$style.button, $style.secondary]" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </button>
        <button
          :class="[$style.button, props.danger ? $style.danger : $style.primary]"
          @click="emit('confirm')"
        >
          {{ props.confirmLabel }}
        </button>
      </div>
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
}

/* A pool of light at the top of the card, falling from the window's spotlight. */
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

.actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 1.5rem;
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

.danger {
  background: var(--c-error-bg);
  color: var(--c-error-text);
  font-weight: 500;
}

.danger:hover {
  background: rgba(127, 29, 29, 0.55);
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
