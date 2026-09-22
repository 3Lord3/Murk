<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { useEventListener, useLocalStorage } from "@vueuse/core";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { useLibraryStore, type MediaKind } from "../stores/library";
import { errorMessage } from "../i18n/errors";
import TitleBar from "../components/TitleBar.vue";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import KindDialog from "../components/KindDialog.vue";
import Icon from "../components/Icon.vue";
import { mdiDotsVertical, mdiImage, mdiClose, mdiRefresh, mdiRestore, mdiDelete } from "@mdi/js";

const library = useLibraryStore();
const router = useRouter();
const { t } = useI18n();

onMounted(library.refresh);

/** A stable hue derived from the name: colour without artwork. */
function coverTint(name: string) {
  let hue = 0;
  for (let i = 0; i < name.length; i++) hue = (hue * 31 + name.charCodeAt(i)) % 360;
  return {
    light: `hsl(${hue} 55% 42%)`,
    deep: `hsl(${hue} 45% 20%)`,
  };
}

function coverStyle(name: string) {
  const { light, deep } = coverTint(name);
  return { "--tint-light": light, "--tint-deep": deep };
}

function coverInitial(name: string) {
  const ch = name.trim().charAt(0);
  return ch ? ch.toUpperCase() : "?";
}

async function addFolder() {
  // `directory: true` is the privacy rule, not a convenience: a chooser in
  // file mode lists filenames, and "S02E08 - Endings and Beginnings.mkv" is a
  // spoiler the user cannot unsee.
  const picked = await open({ directory: true, multiple: false, title: t("library.dialog.folderTitle") });
  if (typeof picked === "string") pendingFolder.value = picked;
}

// Which shelf the picked folder goes on, asked rather than guessed.
const pendingFolder = ref<string | null>(null);
async function pickKind(kind: MediaKind) {
  const folder = pendingFolder.value;
  pendingFolder.value = null;
  if (!folder) return;
  // Bring the shelf the folder lands on to the front, or the new card would
  // be filtered out and the add would look like it did nothing.
  tab.value = kind;
  await runAction(async () => {
    const { added, skipped } = await library.add(folder, kind);
    // Tell the user if the per-folder cap left titles out.
    if (skipped > 0) library.notice = t("library.folderTooLarge");
    else if (added === 0) library.notice = t("library.nothingAdded");
  });
}

// Prevent a second add/rescan from starting while one is running.
const busy = ref(false);
async function runAction(action: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true;
  try {
    await action();
  } catch (e) {
    library.error = String(e);
  } finally {
    busy.value = false;
  }
}

async function rescanAll() {
  await runAction(() => library.rescanAll());
}

async function watch(seriesId: number) {
  // A rejected promise here used to skip the navigation silently, leaving the
  // button looking broken.
  library.error = null;
  try {
    await invoke("continue_series", { seriesId });
  } catch (e) {
    library.error = String(e);
    return;
  }
  router.push("/watch");
}

// The "…" menu on a card: poster, rescan and removal all live behind it so the
// row stays a single Continue button.
const openMenu = ref<number | null>(null);
function closeMenu() {
  openMenu.value = null;
}
function toggleMenu(seriesId: number) {
  openMenu.value = openMenu.value === seriesId ? null : seriesId;
}
// Any click or right-click *not* on the card itself closes the menu. The card's
// own handlers stop propagation so opening it is not undone by the same event.
useEventListener(window, "click", closeMenu);
useEventListener(window, "contextmenu", closeMenu);

async function choosePoster(seriesId: number) {
  closeMenu();
  const picked = await open({
    multiple: false,
    title: t("library.dialog.posterTitle"),
    filters: [{ name: t("library.dialog.imageFilter"), extensions: ["jpg", "jpeg", "png", "webp"] }],
  });
  if (typeof picked === "string") {
    await library.setPoster(seriesId, picked);
  }
}

async function rescanSeries(seriesId: number) {
  closeMenu();
  await runAction(() => library.rescan(seriesId));
}

// The pending destructive action, or null when no dialog is up. Holding the
// series id here rather than a callback keeps the dialog a plain component
// that only reports "yes" or "no".
type Pending = { kind: "reset" | "remove"; seriesId: number };
const pending = ref<Pending | null>(null);

function askResetProgress(seriesId: number) {
  closeMenu();
  pending.value = { kind: "reset", seriesId };
}

function askRemoveSeries(seriesId: number) {
  closeMenu();
  pending.value = { kind: "remove", seriesId };
}

async function confirmPending() {
  const action = pending.value;
  pending.value = null;
  if (!action) return;
  if (action.kind === "reset") {
    await library.resetProgress(action.seriesId);
  } else {
    await library.remove(action.seriesId);
  }
}

async function clearPoster(seriesId: number) {
  closeMenu();
  await library.clearPoster(seriesId);
}

// The two shelves; the tabs just pick which is in front. The choice sticks
// between sessions, so a movie collector is not sent back to series each time.
const tab = useLocalStorage<MediaKind>("library.tab", "series");
const tabs: MediaKind[] = ["series", "movie"];
const visibleSeries = computed(() => library.series.filter((s) => s.kind === tab.value));

// Arrow keys move between tabs, as a tablist is expected to.
function onTabKey(event: KeyboardEvent) {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  const step = event.key === "ArrowRight" ? 1 : -1;
  tab.value = tabs[(tabs.indexOf(tab.value) + step + tabs.length) % tabs.length];
}

</script>

<template>
  <div :class="$style.page">
    <TitleBar />

    <main :class="$style.main">
      <header :class="$style.header">
        <h1 :class="$style.title">{{ t("library.title") }}</h1>
        <div :class="$style.headerActions">
          <button :class="$style.btn" :disabled="busy" @click="addFolder">{{ t("library.addFolder") }}</button>
          <button :class="$style.btn" :disabled="busy" @click="rescanAll">{{ t("library.rescanAll") }}</button>
          <button :class="$style.btn" @click="router.push('/settings')">{{ t("library.settings") }}</button>
        </div>
      </header>

      <nav :class="$style.tabs" role="tablist" :aria-label="t('library.title')" @keydown="onTabKey">
        <button
          v-for="k in tabs"
          :id="`library-tab-${k}`"
          :key="k"
          :class="[$style.tab, tab === k ? $style.tabActive : null]"
          role="tab"
          :aria-selected="tab === k"
          aria-controls="library-panel"
          :tabindex="tab === k ? 0 : -1"
          @click="tab = k"
        >
          {{ t(`library.tabs.${k}`) }}
        </button>
      </nav>

      <p v-if="library.error" :class="$style.error">{{ errorMessage(library.error) }}</p>
      <p v-if="library.notice" :class="$style.notice">{{ library.notice }}</p>

      <div id="library-panel" role="tabpanel" :aria-labelledby="`library-tab-${tab}`">
        <div v-if="!library.loading && visibleSeries.length === 0" :class="$style.empty">
          <p :class="$style.emptyLead">{{ t("library.empty.lead") }}</p>
          <p :class="$style.emptyHint">{{ t(`library.empty.${tab}Hint`) }}</p>
        </div>

        <ul :class="$style.grid">
        <li
          v-for="s in visibleSeries"
          :key="s.id"
          :class="$style.card"
          @contextmenu.prevent.stop="openMenu = s.id"
        >
          <!-- Clips the glossy sweep ribbon to the card without clipping the
               "…" menu, which may reach over the cover. -->
          <div :class="$style.sheen" aria-hidden="true" />
          <!--
            No poster from a metadata provider: promotional art routinely shows
            a scene from the finale. A cover is only ever a local file, the
            user's own folder art or an embedded one; with none, the card falls
            back to a colour field tinted from the name, with the initial as a
            watermark.
          -->
          <div :class="$style.cover" :style="coverStyle(s.displayName)">
            <img
              v-if="s.poster"
              :class="$style.art"
              :src="s.poster"
              :alt="t('library.coverAlt', { name: s.displayName })"
            />
            <div v-if="s.poster" :class="$style.artShade" aria-hidden="true" />
            <span v-else :class="$style.monogram" aria-hidden="true">
              {{ coverInitial(s.displayName) }}
            </span>
            <h2 :class="$style.name">{{ s.displayName }}</h2>

            <!--
              The whole work, not the episode in hand: every file in the folder
              end to end. A bar and nothing else: no percentage, no "8 of 24",
              nothing that would say how much of the story is left.
            -->
            <div
              v-if="s.progress !== undefined"
              :class="$style.progressTrack"
              role="progressbar"
              :aria-label="t('library.progressAlt', { name: s.displayName })"
            >
              <div :class="$style.progressFill" :style="{ width: `${s.progress * 100}%` }" />
            </div>
          </div>

          <div :class="$style.cardActions">
            <!--
              One button. No "12/24", no episode list, no "next: S03E09".
              `inProgress` only distinguishes the wording, never the place.
            -->
            <button :class="[$style.btnPrimary, $style.grow]" @click="watch(s.id)">
              {{ t(s.inProgress ? "library.continue" : "library.start") }}
            </button>

            <div :class="$style.menuWrap">
              <button
                :class="[$style.btnIcon, openMenu === s.id ? $style.btnActive : null]"
                :title="t('library.more')"
                @click.stop="toggleMenu(s.id)"
              >
                <Icon :class="$style.icon" :path="mdiDotsVertical" />
              </button>

              <div v-if="openMenu === s.id" :class="$style.menu" @click.stop>
                <button :class="$style.menuItem" @click="choosePoster(s.id)">
                  <Icon :class="$style.menuIcon" :path="mdiImage" />
                  {{ t(s.poster ? "library.menu.replacePoster" : "library.menu.choosePoster") }}
                </button>
                <button v-if="s.poster" :class="$style.menuItem" @click="clearPoster(s.id)">
                  <Icon :class="$style.menuIcon" :path="mdiClose" />
                  {{ t("library.menu.clearPoster") }}
                </button>
                <button :class="$style.menuItem" @click="rescanSeries(s.id)">
                  <Icon :class="$style.menuIcon" :path="mdiRefresh" />
                  {{ t("library.menu.rescan") }}
                </button>
                <button
                  :class="$style.menuItem"
                  :disabled="!s.hasProgress"
                  @click="askResetProgress(s.id)"
                >
                  <Icon :class="$style.menuIcon" :path="mdiRestore" />
                  {{ t("library.menu.resetProgress") }}
                </button>
                <button :class="[$style.menuItem, $style.menuItemDanger]" @click="askRemoveSeries(s.id)">
                  <Icon :class="$style.menuIcon" :path="mdiDelete" />
                  {{ t("library.menu.remove") }}
                </button>
              </div>
            </div>
          </div>
        </li>
        </ul>
      </div>
    </main>

    <ConfirmDialog
      v-if="pending"
      :title="t(`library.dialog.${pending.kind}Title`)"
      :body="t(`library.dialog.${pending.kind}Body`)"
      :confirm-label="t(`library.dialog.${pending.kind}Confirm`)"
      :danger="pending.kind === 'remove'"
      @confirm="confirmPending"
      @cancel="pending = null"
    />

    <KindDialog v-if="pendingFolder !== null" @select="pickKind" @cancel="pendingFolder = null" />
  </div>
</template>

<style module>
.page {
  display: flex;
  height: 100%;
  flex-direction: column;
}

.main {
  flex: 1;
  overflow-y: auto;
  padding: 0 1rem 2.5rem;
}

.header {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.75rem;
  margin-bottom: 2rem;
}

.title {
  font-size: 1.5rem;
  font-weight: 500;
  letter-spacing: -0.01em;
  color: var(--c-text-strong);
}

.headerActions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.tabs {
  display: inline-flex;
  gap: 0.25rem;
  margin: 1.5rem 0 1.5rem;
  border-radius: var(--r-md);
  background: var(--c-glass);
  box-shadow: inset 0 0 0 1px var(--c-hairline-soft);
  padding: 0.25rem;
}

.tab {
  border-radius: var(--r-sm);
  padding: 0.375rem 1rem;
  font-size: 0.875rem;
  color: var(--c-text-muted);
  transition: background-color var(--t-fast), color var(--t-fast), box-shadow var(--t-fast);
}

.tab:hover {
  color: var(--c-text);
}

.tabActive,
.tabActive:hover {
  background: var(--c-surface-raised);
  box-shadow: inset 0 0 0 1px var(--c-hairline);
  color: var(--c-text-strong);
}

.btn {
  border-radius: var(--r-md);
  background: var(--c-glass);
  box-shadow: inset 0 0 0 1px var(--c-hairline-soft);
  padding: 0.375rem 0.75rem;
  font-size: 0.875rem;
  color: var(--c-text);
  transition: background-color var(--t-fast), box-shadow var(--t-fast);
}

.btn:hover {
  background: rgba(255, 255, 255, 0.1);
  box-shadow: inset 0 0 0 1px var(--c-hairline);
}

.btnPrimary {
  border-radius: var(--r-md);
  background: linear-gradient(180deg, #f4f7fb, #d9e1ec);
  box-shadow:
    0 4px 16px rgba(226, 232, 240, 0.22),
    inset 0 1px 0 #fff;
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--c-accent-text);
  transition: background-color var(--t-fast), box-shadow var(--t-fast);
}

.btnPrimary:hover {
  background: linear-gradient(180deg, #fff, #e6edf6);
  box-shadow:
    0 6px 24px rgba(226, 232, 240, 0.32),
    inset 0 1px 0 #fff;
}

.btnIcon {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 2.25rem;
  width: 2.25rem;
  flex: none;
  border-radius: var(--r-md);
  padding: 0;
  color: var(--c-text-muted);
  transition: background-color var(--t-fast), color var(--t-fast), box-shadow var(--t-fast), opacity var(--t-fast);
}

.btnIcon:hover {
  background: var(--c-glass);
  box-shadow: inset 0 0 0 1px var(--c-hairline-soft);
  color: var(--c-text);
}

.btnActive,
.btnIcon.btnActive:hover {
  background: var(--c-glass);
  box-shadow: inset 0 0 0 1px var(--c-hairline);
  color: var(--c-text-strong);
}

.icon {
  width: 1.375rem;
  height: 1.375rem;
  fill: currentColor;
}

.menuWrap {
  position: relative;
}

/* The menu opens *upward*, over the cover. The card no longer clips its
   descendants (the sweep has its own overlay), so it can hold five items. */
.menu {
  position: absolute;
  bottom: calc(100% + 0.375rem);
  right: 0;
  z-index: 5;
  display: flex;
  min-width: 14rem;
  flex-direction: column;
  gap: 0.125rem;
  border-radius: var(--r-md);
  background: rgba(20, 25, 36, 0.94);
  box-shadow:
    inset 0 0 0 1px var(--c-hairline),
    0 12px 32px rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(14px);
  padding: 0.25rem;
}

.menuItem {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  border-radius: var(--r-sm);
  padding: 0.375rem 0.625rem;
  font-size: 0.8125rem;
  line-height: 1.2;
  text-align: left;
  color: var(--c-text);
  transition: background-color var(--t-fast), color var(--t-fast);
}

.menuItem:hover {
  background: var(--c-glass);
  color: var(--c-text-strong);
}

.menuItem:disabled {
  opacity: 0.45;
  cursor: default;
}

.menuItem:disabled:hover {
  background: none;
  color: var(--c-text);
}

.menuItemDanger:hover {
  background: rgba(127, 29, 29, 0.55);
  color: #fca5a5;
}

.menuIcon {
  width: 1.125rem;
  height: 1.125rem;
  flex: none;
  fill: currentColor;
}

.grow {
  flex: 1;
}

.error {
  margin-bottom: 1.5rem;
  border-radius: var(--r-md);
  background: var(--c-error-bg);
  padding: 0.75rem 1rem;
  font-size: 0.875rem;
  color: #fecaca;
}

.notice {
  margin-bottom: 1.5rem;
  border-radius: var(--r-md);
  background: var(--c-glass);
  box-shadow: inset 0 0 0 1px var(--c-hairline-soft);
  padding: 0.75rem 1rem;
  font-size: 0.875rem;
  color: var(--c-text);
}

.empty {
  margin-top: 6rem;
  text-align: center;
  color: var(--c-text-muted);
}

.emptyLead {
  font-size: 1.125rem;
}

.emptyHint {
  margin-top: 0.5rem;
  font-size: 0.875rem;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 200px), 1fr));
  gap: 1rem;
}

.card {
  position: relative;
  display: flex;
  flex-direction: column;
  border-radius: var(--r-lg);
  background: linear-gradient(
    165deg,
    rgba(255, 255, 255, 0.07) 0%,
    rgba(255, 255, 255, 0.02) 42%,
    rgba(15, 19, 27, 0.45) 100%
  );
  box-shadow:
    inset 0 0 0 1px var(--c-hairline-soft),
    inset 0 1px 0 rgba(255, 255, 255, 0.09);
  backdrop-filter: blur(14px);
  transition: transform var(--t-fast), box-shadow var(--t-fast);
  animation: rise 0.5s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}

.card:nth-child(2) {
  animation-delay: 40ms;
}

.card:nth-child(3) {
  animation-delay: 80ms;
}

.card:nth-child(4) {
  animation-delay: 120ms;
}

.card:nth-child(5) {
  animation-delay: 160ms;
}

.card:nth-child(6) {
  animation-delay: 200ms;
}

/* The glossy sweep: a skewed ribbon of light passes over the card on hover.
   `left` animates a single pseudo-element, so only that element re-paints. */
.sheen {
  pointer-events: none;
  position: absolute;
  inset: 0;
  z-index: 2;
  overflow: hidden;
  border-radius: inherit;
}

.sheen::before {
  content: "";
  position: absolute;
  top: -20%;
  bottom: -20%;
  left: -60%;
  width: 45%;
  background: linear-gradient(105deg, transparent, rgba(255, 255, 255, 0.16), transparent);
  transform: skewX(-18deg);
  transition: left var(--t-sweep);
}

.card:hover .sheen::before {
  left: 130%;
}

.card:hover {
  transform: translateY(-2px);
  box-shadow:
    inset 0 0 0 1px rgba(255, 255, 255, 0.18),
    inset 0 1px 0 rgba(255, 255, 255, 0.12),
    0 14px 32px rgba(0, 0, 0, 0.5);
}

@keyframes rise {
  from {
    opacity: 0;
    transform: translateY(10px);
  }
}

.cover {
  position: relative;
  z-index: 1;
  display: flex;
  height: 8rem;
  align-items: flex-end;
  overflow: hidden;
  border-radius: var(--r-lg) var(--r-lg) 0 0;
  background:
    radial-gradient(130% 110% at 85% -15%, var(--tint-light), transparent 55%),
    linear-gradient(160deg, var(--tint-deep), #0d1017 78%);
  padding: 1rem;
}

.art {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

/* Keeps the title readable over bright artwork. */
.artShade {
  pointer-events: none;
  position: absolute;
  inset: 0;
  background: linear-gradient(to top, rgba(5, 7, 10, 0.72), transparent 45%);
}

/* A face for the card without showing any artwork. */
.monogram {
  pointer-events: none;
  position: absolute;
  top: 0.5rem;
  right: 0.875rem;
  font-size: 3.25rem;
  font-weight: 600;
  line-height: 1;
  letter-spacing: -0.02em;
  color: rgba(255, 255, 255, 0.16);
}

/* A soft pool of light on the cover, from the window's spotlight. */
.cover::after {
  content: "";
  pointer-events: none;
  position: absolute;
  inset: 0;
  background: radial-gradient(120% 90% at 50% 0%, rgba(255, 255, 255, 0.15), transparent 55%);
}

.name {
  position: relative;
  z-index: 2;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  font-size: 1rem;
  font-weight: 500;
  color: var(--c-text-strong);
  text-shadow: 0 1px 3px rgba(0, 0, 0, 0.7);
}

/* Pinned to the bottom edge of the cover rather than laid out with the title:
   `.cover` is a flex row, and a bar with no content in it is a bar 0px wide. */
.progressTrack {
  position: absolute;
  z-index: 3;
  right: 0;
  bottom: 0;
  left: 0;
  height: 3px;
  background: rgba(255, 255, 255, 0.22);
}

.progressFill {
  height: 100%;
  border-radius: inherit;
  background: var(--c-accent);
}

.cardActions {
  position: relative;
  z-index: 3;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.75rem;
}

@media (min-width: 640px) {
  .main {
    padding-inline: 2rem;
  }

  .grid {
    gap: 1.25rem;
  }
}
</style>
