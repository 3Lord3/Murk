import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface SeriesCard {
  id: number;
  displayName: string;
  /** Whether Continue resumes rather than starts. A boolean, never a position. */
  inProgress: boolean;
  /** Whether any progress is stored, including a series watched to the end. */
  hasProgress: boolean;
  /**
   * How far the whole work has been watched, 0 to 1: the folder end to end,
   * not one episode. Absent unless the profile shows progress bars.
   */
  progress?: number;
  /** Cover as a data URL, when the series has one. */
  poster: string | null;
}

export const useLibraryStore = defineStore("library", () => {
  const series = ref<SeriesCard[]>([]);
  const loading = ref(false);
  /** The failure *code* from the last command, never a ready-made sentence. */
  const error = ref<string | null>(null);

  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      series.value = await invoke<SeriesCard[]>("list_series");
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function add(path: string) {
    await invoke("add_series", { path });
    await refresh();
  }

  async function remove(seriesId: number) {
    await invoke("remove_series", { seriesId });
    await refresh();
  }

  async function rescan(seriesId: number) {
    await invoke("rescan_series", { seriesId });
    await refresh();
  }

  async function resetProgress(seriesId: number) {
    await invoke("reset_progress", { seriesId });
    await refresh();
  }

  async function setPoster(seriesId: number, path: string) {
    await invoke("set_series_poster", { seriesId, path });
    await refresh();
  }

  async function clearPoster(seriesId: number) {
    await invoke("clear_series_poster", { seriesId });
    await refresh();
  }

  return { series, loading, error, refresh, add, remove, rescan, resetProgress, setPoster, clearPoster };
});
