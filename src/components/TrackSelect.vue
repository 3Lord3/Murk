<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from "vue";

export type TrackOption = { value: string; label: string };

const props = defineProps<{
  /** Stable identity of the chosen option; `undefined` means nothing matched. */
  modelValue: string | undefined;
  options: TrackOption[];
  label: string;
}>();

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const uid = useId();
const root = ref<HTMLElement | null>(null);
const list = ref<HTMLElement | null>(null);
const open = ref(false);
/** Which row the keyboard is on while the list is open; -1 until it is. */
const active = ref(-1);

const selectedIndex = computed(() => props.options.findIndex((o) => o.value === props.modelValue));

function show() {
  open.value = true;
  active.value = selectedIndex.value >= 0 ? selectedIndex.value : 0;
  // The listener is added on the next frame so the click that opened the list
  // does not immediately close it again on its way up the document.
  nextTick(() => {
    document.addEventListener("pointerdown", onOutside, true);
    list.value?.focus();
  });
}

function hide(refocus = true) {
  if (!open.value) return;
  open.value = false;
  active.value = -1;
  document.removeEventListener("pointerdown", onOutside, true);
  if (refocus) (root.value?.querySelector("button") as HTMLElement | undefined)?.focus();
}

function onOutside(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node)) hide(false);
}

function choose(index: number) {
  const option = props.options[index];
  if (option) emit("update:modelValue", option.value);
  hide();
}

function move(delta: number) {
  if (!open.value) return show();
  const count = props.options.length;
  if (count === 0) return;
  active.value = (active.value + delta + count) % count;
}

function onKeydown(event: KeyboardEvent) {
  switch (event.key) {
    case "ArrowDown":
      move(1);
      break;
    case "ArrowUp":
      move(-1);
      break;
    case "Home":
      active.value = 0;
      break;
    case "End":
      active.value = props.options.length - 1;
      break;
    case "Enter":
    case " ":
      open.value ? choose(active.value) : show();
      break;
    case "Escape":
      hide();
      break;
    case "Tab":
      hide(false);
      return;
    default:
      return;
  }
  event.preventDefault();
  // WatchView listens for the same keys on `window`: without this, Escape would
  // also leave playback and the arrows would also move the volume.
  event.stopPropagation();
}

// A track list can change under the panel mid-playback, leaving an open list
// pointing at rows that no longer exist. Compared by value, not by identity:
// the parent rebuilds this array on every tick without its contents moving.
watch(
  () => props.options.map((o) => o.value).join("\u0000"),
  () => hide(false),
);

onBeforeUnmount(() => document.removeEventListener("pointerdown", onOutside, true));
</script>

<template>
  <div ref="root" :class="$style.wrap" @keydown="onKeydown">
    <button
      type="button"
      :class="[$style.trigger, open && $style.triggerOpen]"
      :title="label"
      :aria-label="label"
      :aria-expanded="open"
      aria-haspopup="listbox"
      @click="open ? hide() : show()"
    >
      <!-- The parent's glyph, in the same 24x24 box as the rest of the row. -->
      <slot name="icon" />
    </button>

    <!--
      The list opens upward: the panel sits at the bottom of the window, so
      downward is off-screen.
    -->
    <ul
      v-if="open"
      ref="list"
      :class="$style.menu"
      role="listbox"
      tabindex="-1"
      :aria-label="label"
      :aria-activedescendant="active >= 0 ? `${uid}-opt-${active}` : undefined"
    >
      <li
        v-for="(option, i) in options"
        :id="`${uid}-opt-${i}`"
        :key="option.value"
        role="option"
        :aria-selected="i === selectedIndex"
        :class="[$style.option, i === active && $style.optionActive]"
        @pointerenter="active = i"
        @click="choose(i)"
      >
        <svg :class="$style.check" viewBox="0 0 24 24" aria-hidden="true">
          <path v-if="i === selectedIndex" d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z" />
        </svg>
        <span :class="$style.optionLabel">{{ option.label }}</span>
      </li>
    </ul>
  </div>
</template>

<style module>
.wrap {
  position: relative;
}

/* The same box and hover as the plain icon buttons in the row. */
.trigger {
  display: flex;
  height: 2.25rem;
  min-width: 2.25rem;
  align-items: center;
  justify-content: center;
  border-radius: var(--r-md);
  padding: 0 0.5rem;
  color: var(--c-text);
  transition: background-color var(--t-fast);
}

.trigger:hover {
  background: var(--c-veil);
}

.triggerOpen {
  background: var(--c-veil-strong);
  color: var(--c-text-strong);
}

.menu {
  position: absolute;
  bottom: calc(100% + 0.375rem);
  left: 50%;
  transform: translateX(-50%);
  z-index: 10;
  min-width: 9rem;
  max-width: 16rem;
  max-height: 14rem;
  overflow-y: auto;
  border-radius: var(--r-lg);
  border: 1px solid var(--c-hairline);
  background: var(--c-panel);
  padding: 0.25rem;
  backdrop-filter: blur(12px);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  outline: none;
}

.option {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  border-radius: var(--r-sm);
  padding: 0.375rem 0.5rem;
  font-size: 0.75rem;
  color: var(--c-text);
  cursor: default;
  white-space: nowrap;
}

.optionActive {
  background: var(--c-veil);
  color: var(--c-text-strong);
}

.check {
  width: 0.875rem;
  height: 0.875rem;
  flex: none;
  fill: currentColor;
}

.optionLabel {
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
