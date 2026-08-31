import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export type PeekMode = "disabled" | "coarse" | "confirmed";

export interface HidingProfile {
  id: string;
  hideTitle: boolean;
  hideEpisodeNumber: boolean;
  hideSeasonNumber: boolean;
  hideEpisodeCount: boolean;
  hideProgressBar: boolean;
  hidePosition: boolean;
  hideDuration: boolean;
  hideRemaining: boolean;
  hideChapters: boolean;
  hideArtwork: boolean;
  hideNextUp: boolean;
  peek: PeekMode;
}

export const useProfileStore = defineStore("profile", () => {
  const current = ref<HidingProfile | null>(null);
  const available = ref<HidingProfile[]>([]);

  async function refresh() {
    current.value = await invoke<HidingProfile>("get_profile");
    available.value = await invoke<HidingProfile[]>("list_profiles");
  }

  async function select(profileId: string) {
    current.value = await invoke<HidingProfile>("set_profile", { profileId });
  }

  return { current, available, refresh, select };
});
