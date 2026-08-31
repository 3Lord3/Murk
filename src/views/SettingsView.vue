<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { useProfileStore } from "../stores/profile";
import TitleBar from "../components/TitleBar.vue";
import {
  LOCALE_NAMES,
  initLocale,
  resolveLocale,
  setLocale,
  type Locale,
  type LocaleSetting,
} from "../i18n";

const profile = useProfileStore();
const router = useRouter();
const { t, te } = useI18n();

const locale = ref<LocaleSetting>("system");
const LOCALE_OPTIONS: LocaleSetting[] = [
  "system",
  ...(Object.keys(LOCALE_NAMES) as Locale[]),
];

onMounted(async () => {
  await profile.refresh();
  locale.value = await initLocale();
});

async function chooseLocale(setting: LocaleSetting) {
  locale.value = setting;
  await setLocale(setting);
}

</script>

<template>
  <div :class="$style.page">
    <TitleBar />

    <main :class="$style.main">
      <header :class="$style.header">
        <h1 :class="$style.title">{{ t("settings.title") }}</h1>
        <button :class="$style.back" @click="router.push('/')">{{ t("settings.back") }}</button>
      </header>

      <h2 :class="$style.section">{{ t("settings.profiles.heading") }}</h2>
      <p :class="$style.sectionNote">{{ t("settings.profiles.note") }}</p>

      <ul :class="$style.list">
        <li v-for="p in profile.available" :key="p.id">
          <button
            :class="[$style.option, profile.current?.id === p.id ? $style.selected : null]"
            @click="profile.select(p.id)"
          >
            <div :class="$style.optionHead">
              <span :class="$style.optionName">
                {{ te(`profiles.${p.id}.title`) ? t(`profiles.${p.id}.title`) : p.id }}
              </span>
              <span v-if="profile.current?.id === p.id" :class="$style.badge">
                {{ t("settings.profiles.selected") }}
              </span>
            </div>
            <p :class="$style.blurb">
              {{ te(`profiles.${p.id}.blurb`) ? t(`profiles.${p.id}.blurb`) : "" }}
            </p>
            <p :class="$style.meta">
              {{
                t("settings.profiles.meta", {
                  bar: t(p.hideProgressBar ? "settings.profiles.barHidden" : "settings.profiles.barShown"),
                  peek: t(`settings.peekModes.${p.peek}`),
                })
              }}
            </p>
          </button>
        </li>
      </ul>

      <h2 :class="[$style.section, $style.spaced]">{{ t("settings.language.heading") }}</h2>
      <p :class="$style.sectionNote">{{ t("settings.language.note") }}</p>

      <ul :class="$style.list">
        <li v-for="option in LOCALE_OPTIONS" :key="option">
          <button
            :class="[$style.option, locale === option ? $style.selected : null]"
            @click="chooseLocale(option)"
          >
            <div :class="$style.optionHead">
              <span :class="$style.optionName">
                {{ option === "system" ? t("settings.language.system") : LOCALE_NAMES[option] }}
              </span>
              <span v-if="locale === option" :class="$style.badge">
                {{ t("settings.language.selected") }}
              </span>
            </div>
            <p v-if="option === 'system'" :class="$style.meta">
              {{ t("settings.language.systemResolved", { language: LOCALE_NAMES[resolveLocale("system")] }) }}
            </p>
          </button>
        </li>
      </ul>

      <h2 :class="[$style.section, $style.spaced]">{{ t("settings.keys.heading") }}</h2>
      <dl :class="$style.keys">
        <dt :class="$style.key">{{ t("settings.keys.space") }}</dt><dd>{{ t("settings.keys.pause") }}</dd>
        <dt :class="$style.key">{{ t("settings.keys.arrows") }}</dt><dd>{{ t("settings.keys.seek10") }}</dd>
        <dt :class="$style.key">{{ t("settings.keys.shiftArrows") }}</dt><dd>{{ t("settings.keys.seek60") }}</dd>
        <dt :class="$style.key">{{ t("settings.keys.ctrlArrows") }}</dt><dd>{{ t("settings.keys.seek300") }}</dd>
        <dt :class="$style.key">{{ t("settings.keys.verticalArrows") }}</dt><dd>{{ t("settings.keys.volume") }}</dd>
        <dt :class="$style.key">{{ t("settings.keys.f") }}</dt><dd>{{ t("settings.keys.fullscreen") }}</dd>
        <dt :class="$style.key">{{ t("settings.keys.question") }}</dt><dd>{{ t("settings.keys.peek") }}</dd>
        <dt :class="$style.key">{{ t("settings.keys.escape") }}</dt><dd>{{ t("settings.keys.leave") }}</dd>
      </dl>
    </main>
  </div>
</template>

<style module>
.page {
  display: flex;
  height: 100%;
  flex-direction: column;
}

.main {
  margin: 0 auto;
  width: 100%;
  max-width: 42rem;
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

.back {
  border-radius: var(--r-md);
  background: var(--c-glass);
  box-shadow: inset 0 0 0 1px var(--c-hairline-soft);
  padding: 0.375rem 0.75rem;
  font-size: 0.875rem;
  color: var(--c-text);
  transition: background-color var(--t-fast), box-shadow var(--t-fast);
}

.back:hover {
  background: rgba(255, 255, 255, 0.1);
  box-shadow: inset 0 0 0 1px var(--c-hairline);
}

.section {
  margin-bottom: 0.25rem;
  font-size: 0.875rem;
  font-weight: 500;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--c-text-muted);
}

.spaced {
  margin-top: 2.5rem;
}

.sectionNote {
  margin-bottom: 1rem;
  font-size: 0.875rem;
  color: var(--c-text-faint);
}

.list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.option {
  position: relative;
  width: 100%;
  overflow: hidden;
  border-radius: var(--r-lg);
  padding: 1rem;
  text-align: left;
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
  transition: background-color var(--t-fast), box-shadow var(--t-fast);
}

/* Re-selecting the current profile does nothing, so the selected option is not
   an action and drops the pointer the global button rule gives it. */
.selected,
.selected * {
  cursor: default;
}

/* Same glossy sweep as the library cards. */
.option::before {
  content: "";
  pointer-events: none;
  position: absolute;
  top: -20%;
  bottom: -20%;
  left: -60%;
  width: 45%;
  background: linear-gradient(105deg, transparent, rgba(255, 255, 255, 0.16), transparent);
  transform: skewX(-18deg);
  transition: left var(--t-sweep);
}

.option:hover::before {
  left: 130%;
}

.option:hover {
  box-shadow:
    inset 0 0 0 1px rgba(255, 255, 255, 0.18),
    inset 0 1px 0 rgba(255, 255, 255, 0.12);
}

.selected,
.selected:hover {
  background: linear-gradient(
    165deg,
    rgba(42, 51, 72, 0.75) 0%,
    rgba(26, 32, 48, 0.65) 55%,
    rgba(15, 19, 27, 0.5) 100%
  );
  box-shadow:
    inset 0 0 0 1px var(--c-hairline-strong),
    inset 0 1px 0 rgba(255, 255, 255, 0.14),
    0 10px 28px rgba(0, 0, 0, 0.45);
}

.optionHead {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.optionName {
  font-weight: 500;
  color: var(--c-text-strong);
}

.badge {
  font-size: 0.75rem;
  color: var(--c-text-muted);
}

.blurb {
  margin-top: 0.25rem;
  font-size: 0.875rem;
  color: var(--c-text-muted);
}

.meta {
  margin-top: 0.5rem;
  font-size: 0.75rem;
  color: var(--c-text-faint);
}

.keys {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.5rem 1rem;
  font-size: 0.875rem;
  color: var(--c-text-muted);
}

.key {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--c-text);
}

@media (min-width: 640px) {
  .main {
    padding-inline: 2rem;
  }

  .keys {
    column-gap: 1.5rem;
  }
}
</style>
