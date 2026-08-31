import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface TrackView {
  id: number;
  /** Absent when the backend judged the title unsafe to show. */
  title?: string;
  lang?: string;
  selected: boolean;
}

/**
 * The shape the backend sends.
 *
 * Every hideable field is optional, which is the type system enforcing the
 * same rule as the server projection: `positionSec` is `number | undefined`,
 * so `state.positionSec.toFixed(0)` does not compile.
 */
export interface PlaybackView {
  paused: boolean;
  idle: boolean;
  volume: number;
  audioTracks: TrackView[];
  subtitleTracks: TrackView[];

  positionSec?: number;
  durationSec?: number;
  remainingSec?: number;
  progress?: number;
  episodeLabel?: string;
  seasonNumber?: number;
  episodeNumber?: number;
  episodeCount?: number;
}

const EMPTY: PlaybackView = {
  paused: true,
  idle: true,
  volume: 100,
  audioTracks: [],
  subtitleTracks: [],
};

export const usePlaybackStore = defineStore("playback", () => {
  const view = ref<PlaybackView>(EMPTY);
  /** A peeked value, held briefly and then forgotten. */
  const peeked = ref<string | null>(null);
  let peekTimer: number | undefined;

  /**
   * Where unmuting goes back to. Mute is a frontend gesture: the backend knows
   * only about a volume, so the pre-mute level is remembered here.
   */
  const preMuteVolume = ref(EMPTY.volume);
  const isMuted = computed(() => view.value.volume === 0);

  const isPlaying = computed(() => !view.value.paused && !view.value.idle);
  /** True only when the profile actually sent a bar to draw. */
  const hasProgressBar = computed(() => view.value.progress !== undefined);

  async function refresh() {
    view.value = await invoke<PlaybackView>("get_playback");
  }

  async function subscribe() {
    await refresh();
    await listen<PlaybackView>("murk://playback", (event) => {
      view.value = event.payload;
    });
  }

  async function toggleMute() {
    if (isMuted.value) {
      // A volume of zero remembered from a previous session would unmute into
      // silence, so fall back to the default level.
      const restored = preMuteVolume.value > 0 ? preMuteVolume.value : EMPTY.volume;
      await invoke("set_volume", { volume: restored });
      return;
    }
    preMuteVolume.value = view.value.volume;
    await invoke("set_volume", { volume: 0 });
  }

  function showPeek(text: string, seconds = 10) {
    peeked.value = text;
    window.clearTimeout(peekTimer);
    // Short-lived on purpose: leaving it up would turn a peek into a readout.
    peekTimer = window.setTimeout(() => (peeked.value = null), seconds * 1000);
  }

  function clearPeek() {
    window.clearTimeout(peekTimer);
    peeked.value = null;
  }

  return { view, peeked, isPlaying, isMuted, hasProgressBar, toggleMute, refresh, subscribe, showPeek, clearPeek };
});
