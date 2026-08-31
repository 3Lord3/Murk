<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";
import { usePlaybackStore } from "../stores/playback";
import TrackSelect, { type TrackOption } from "./TrackSelect.vue";
import { useProfileStore } from "../stores/profile";

const playback = usePlaybackStore();
const profile = useProfileStore();
const { t } = useI18n();

defineEmits<{ leave: []; peek: [] }>();

const canPeek = computed(() => profile.current !== null && profile.current.peek !== "disabled");

// `progress` arrives only under a profile that permits a bar. Otherwise the
// field is not in the payload, so there is nothing to draw.
const progressPercent = computed(() =>
  playback.view.progress === undefined ? null : playback.view.progress * 100,
);

const VOLUME_MAX = 130;

// The fill is painted by hand instead of by `accent-color`, so it has to know
// where the thumb centre sits: 0 at the left edge of travel, 1 at the right.
const volumeFraction = computed(() =>
  Math.min(Math.max(playback.view.volume / VOLUME_MAX, 0), 1),
);

async function setVolume(event: Event) {
  const value = Number((event.target as HTMLInputElement).value);
  await invoke("set_volume", { volume: value });
}

async function pickTrack(kind: "audio" | "subtitle", raw: string) {
  await invoke("set_track", { kind, id: raw === "off" ? null : Number(raw) });
}

/** A track with no title left after sanitising gets a positional label. */
function trackLabel(track: { id: number; title?: string; lang?: string }, index: number) {
  if (track.title) return track.lang ? `${track.title} · ${track.lang}` : track.title;
  if (track.lang) return track.lang;
  return t("controls.trackFallback", { number: index + 1 });
}

function options(tracks: { id: number; title?: string; lang?: string }[]): TrackOption[] {
  return tracks.map((track, i) => ({ value: String(track.id), label: trackLabel(track, i) }));
}

const audioOptions = computed(() => options(playback.view.audioTracks));
const subtitleOptions = computed<TrackOption[]>(() => [
  { value: "off", label: t("controls.noSubtitles") },
  ...options(playback.view.subtitleTracks),
]);

/** `off` is the honest answer when no subtitle track reports itself selected. */
const audioValue = computed(() => {
  const selected = playback.view.audioTracks.find((track) => track.selected);
  return selected === undefined ? undefined : String(selected.id);
});
const subtitleValue = computed(() => {
  const selected = playback.view.subtitleTracks.find((track) => track.selected);
  return selected === undefined ? "off" : String(selected.id);
});
</script>

<template>
  <div :class="$style.dock">
    <div :class="$style.panel">
      <!--
        Where a seek bar normally lives. Under the default profiles this space
        stays empty on purpose: an empty strip is the visible form of the
        promise the program makes.
      -->
      <div v-if="progressPercent !== null" :class="$style.track">
        <div :class="$style.fill" :style="{ width: `${progressPercent}%` }" />
      </div>

      <div :class="$style.row">
        <button
          :class="$style.ctl"
          :title="playback.view.paused ? t('controls.play') : t('controls.pause')"
          @click="invoke('play_pause')"
        >
          <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
            <path v-if="playback.view.paused" d="M8 5v14l11-7z" />
            <path v-else d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" />
          </svg>
        </button>

        <button :class="$style.ctl" :title="t('controls.back10')" @click="invoke('seek_relative', { deltaSec: -10 })">
          <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M12 5V1L7 6l5 5V7c3.31 0 6 2.69 6 6s-2.69 6-6 6-6-2.69-6-6H4c0 4.42 3.58 8 8 8s8-3.58 8-8-3.58-8-8-8z"
            />
          </svg>
        </button>
        <button :class="$style.ctl" :title="t('controls.forward10')" @click="invoke('seek_relative', { deltaSec: 10 })">
          <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M12 5V1l5 5-5 5V7c-3.31 0-6 2.69-6 6s2.69 6 6 6 6-2.69 6-6h2c0 4.42-3.58 8-8 8s-8-3.58-8-8 3.58-8 8-8z"
            />
          </svg>
        </button>

        <div :class="$style.volume">
          <button
            :class="[$style.ctl, $style.muteBtn, playback.isMuted && $style.muted]"
            :title="playback.isMuted ? t('controls.unmute') : t('controls.mute')"
            :aria-pressed="playback.isMuted"
            @click="playback.toggleMute()"
          >
            <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
              <path
                v-if="playback.isMuted"
                d="M12 4L9.91 6.09 12 8.18M4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06a8.99 8.99 0 0 0 3.69-1.81L19.73 21 21 19.73 12 10.73M19 12c0 .94-.2 1.82-.54 2.64l1.51 1.51A8.8 8.8 0 0 0 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71m-2.5 0c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.05-.2.05-.42.05-.63z"
              />
              <path
                v-else
                d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"
              />
            </svg>
          </button>
          <input
            :class="$style.slider"
            type="range"
            min="0"
            :max="VOLUME_MAX"
            :style="{ '--pct': volumeFraction }"
            step="1"
            :value="playback.view.volume"
            :aria-label="t('controls.volume')"
            @input="setVolume"
          />
        </div>

        <TrackSelect
          v-if="playback.view.audioTracks.length > 1"
          :model-value="audioValue"
          :options="audioOptions"
          :label="t('controls.audioTrack')"
          @update:model-value="pickTrack('audio', $event)"
        >
          <template #icon>
            <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M12 3v10.55A4 4 0 1 0 14 17V7h4V3h-6z"
              />
            </svg>
          </template>
        </TrackSelect>

        <TrackSelect
          v-if="playback.view.subtitleTracks.length > 0"
          :model-value="subtitleValue"
          :options="subtitleOptions"
          :label="t('controls.subtitles')"
          @update:model-value="pickTrack('subtitle', $event)"
        >
          <template #icon>
            <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M20 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2zM4 12h4v2H4v-2zm10 6H4v-2h10v2zm6 0h-4v-2h4v2zm0-4H10v-2h10v2z"
              />
            </svg>
          </template>
        </TrackSelect>

        <div :class="$style.trailing">
          <button v-if="canPeek" :class="$style.ctl" :title="t('controls.peek')" @click="$emit('peek')">
            <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm1 17h-2v-2h2v2zm2.07-7.75l-.9.92C13.45 12.9 13 13.5 13 15h-2v-.5c0-1.1.45-2.1 1.17-2.83l1.24-1.26A2 2 0 1 0 10 9H8a4 4 0 1 1 8 0c0 .88-.36 1.68-.93 2.25z"
              />
            </svg>
          </button>
          <button :class="$style.ctl" :title="t('controls.fullscreen')" @click="invoke('toggle_fullscreen')">
            <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z" />
            </svg>
          </button>
          <button :class="$style.ctl" :title="t('controls.leave')" @click="$emit('leave')">
            <svg :class="$style.icon" viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
              />
            </svg>
          </button>
        </div>
      </div>

      <p v-if="playback.peeked" :class="$style.peeked">{{ playback.peeked }}</p>
    </div>
  </div>
</template>

<style module>
.dock {
  pointer-events: none;
  position: absolute;
  inset: 0;
}

.panel {
  pointer-events: auto;
  position: absolute;
  left: 50%;
  bottom: 0.5rem;
  transform: translateX(-50%);
  display: flex;
  width: max-content;
  max-width: min(48rem, calc(100% - 1rem));
  flex-direction: column;
  gap: 0.75rem;
  border-radius: var(--r-xl);
  border: 1px solid var(--c-hairline);
  background: var(--c-scrim);
  padding: 0.75rem;
  backdrop-filter: blur(12px);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.1),
    0 12px 32px rgba(0, 0, 0, 0.45);
}

.track {
  height: 0.25rem;
  width: 100%;
  border-radius: 9999px;
  background: rgba(255, 255, 255, 0.15);
}

.fill {
  height: 100%;
  border-radius: 9999px;
  background: rgba(255, 255, 255, 0.7);
}

.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem 0.75rem;
}

.ctl {
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

.ctl:hover {
  background: var(--c-veil);
}

/* Every glyph shares one 24x24 box, so swapping play for pause never nudges
   the row. */
.icon {
  width: 1.375rem;
  height: 1.375rem;
  flex: none;
  fill: currentColor;
}

.volume {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  color: var(--c-text);
}

/* A full-size control like the rest of the row, so the hover surface matches.
   The negative margins take its own padding back, keeping the visible spacing
   as it was when the icon was bare. */
.muteBtn {
  margin: 0 -0.5rem;
}

/* The one icon in the row that leaves the neutral palette. */
.muted {
  color: var(--c-error-text);
}

/* `accent-color` fills the track to its very end at the maximum, which leaves
   the painted bar running past the centre of the thumb. Drawing the fill from
   the same geometry the browser uses to place the thumb keeps the two aligned
   at every value, including the ends. */
.slider {
  --thumb: 0.875rem;
  --fill: calc(var(--thumb) / 2 + var(--pct, 0) * (100% - var(--thumb)));
  width: 4rem;
  height: var(--thumb);
  appearance: none;
  background: linear-gradient(
    to right,
    var(--c-accent) 0 var(--fill),
    rgba(255, 255, 255, 0.25) var(--fill) 100%
  );
  border-radius: 9999px;
  cursor: pointer;
}

.slider::-webkit-slider-runnable-track {
  height: var(--thumb);
  background: transparent;
}

.slider::-moz-range-track {
  height: var(--thumb);
  background: transparent;
}

.slider::-webkit-slider-thumb {
  appearance: none;
  width: var(--thumb);
  height: var(--thumb);
  border-radius: 9999px;
  border: none;
  background: var(--c-text);
}

.slider::-moz-range-thumb {
  width: var(--thumb);
  height: var(--thumb);
  border-radius: 9999px;
  border: none;
  background: var(--c-text);
}

.trailing {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 0.25rem;
}

.peeked {
  text-align: center;
  font-size: 0.875rem;
  color: rgba(253, 230, 138, 0.9);
}

@media (min-width: 640px) {
  .panel {
    bottom: 1.25rem;
    max-width: min(48rem, calc(100% - 2.5rem));
    padding: 1rem;
  }

  .row {
    column-gap: 1rem;
  }

  .slider {
    width: 6rem;
  }

  .trailing {
    gap: 0.5rem;
  }
}
</style>
