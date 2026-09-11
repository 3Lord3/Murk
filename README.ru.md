<div align="center">

<img src="assets/banner.png" alt="Murk, плеер без полосы просмотра" width="720">

**Смотрите сериал, а не таймлайн.**

<a href="https://github.com/3Lord3/Murk/actions"><img alt="build" src="https://img.shields.io/github/actions/workflow/status/3Lord3/Murk/ci.yml?style=for-the-badge&labelColor=0f131b&label=build"></a>
<a href="#%D1%83%D1%81%D1%82%D0%B0%D0%BD%D0%BE%D0%B2%D0%BA%D0%B0"><img alt="version" src="https://img.shields.io/badge/version-0.1.0-c084fc?style=for-the-badge&labelColor=0f131b"></a>
<a href="#%D0%BB%D0%B8%D1%86%D0%B5%D0%BD%D0%B7%D0%B8%D1%8F"><img alt="license" src="https://img.shields.io/badge/license-GPL--2.0--or--later-e2e8f0?style=for-the-badge&labelColor=0f131b"></a>
<a href="https://tauri.app"><img alt="Tauri 2" src="https://img.shields.io/badge/Tauri_2-6d76ff?style=for-the-badge&labelColor=0f131b&logo=tauri&logoColor=white"></a>
<a href="https://www.rust-lang.org"><img alt="Rust backend" src="https://img.shields.io/badge/Rust_backend-5eead4?style=for-the-badge&labelColor=0f131b&logo=rust&logoColor=white"></a>

[English](README.md) · [Русский](README.ru.md)

</div>

### Ваш сериал, без спойлеров

Murk, это настольный плеер для сериалов и фильмов, который отказывается их спойлерить.

Обычные плееры рассказывают сюжет раньше самой сцены. Полоса просмотра
показывает, что до развязки три минуты, заголовок окна называет серию, счётчик
«9/24» сообщает, что сезон только начался. Murk по умолчанию не показывает
ничего из этого. Не замазывает и не прячет за переключателем: этих чисел просто
нет на экране.

<div align="center">

| | |
|---|---|
| <img src="assets/screenshot-library.png" alt="Murk, тихая полка библиотеки" width="520"> | <img src="assets/screenshot-settings.png" alt="Murk, настройки" width="520"> |
| <img src="assets/screenshot-player.png" alt="Murk, плеер без полосы просмотра" width="520"> | <img src="assets/screenshot-peek.png" alt="Murk, панель подглядывания" width="520"> |

</div>

# Возможности

- 🙈 **Без спойлеров.** Ни названия, номера серии и сезона, ни количества
  серий, ни полосы просмотра, ни позиции, длительности и остатка. Скрытое
  остаётся в бэкенде, поэтому его не покажет ни ошибка вёрстки, ни открытые
  devtools, ни случайная строка в логе.
- 👀 **Успею ли досмотреть.** Узнайте успеете ли вы досмотреть фильм или серию за 10-60 минут, не спойлеря точное оставшееся время и текущий момент просмотра.
- 📚 **Тихая библиотека.** Сериал добавляется папкой, а не файлом, потому что
  имя файла само по себе бывает спойлером. У карточки одна кнопка, *Начать* или
  *Продолжить*. Никаких списков серий, подсказок и индикаторов.
- ▶️ **Финал остаётся закрытым.** Следующая серия сразу запускается или появляется окошко об окончании серии — зависит от выбранного профиля
- 🎬 **Обложки, которые ничего не выдают.** Никакого скачанного арта. Обложка,
  это ваш собственный файл или ровное цветовое поле, подобранное по названию
  сериала.
- 🗣️ **Мультиязычность.** Следует языку системы, переопределяется в
  настройках.

## Профили

Профиль решает, поле за полем, что бэкенду разрешено отдать в окно.
Переключается в настройках в любой момент.

| На экране | Полный мрак | Стандарт | Мягкий |
|---|:--:|:--:|:--:|
| Название, сезон и номер серии | скрыто | скрыто | видно |
| Сколько серий в сезоне | скрыто | скрыто | скрыто |
| Полоса просмотра | скрыто | скрыто | видно |
| Позиция, длительность, остаток | скрыто | скрыто | скрыто |
| Метки глав | скрыто | скрыто | скрыто |
| Обложка из вашей папки | скрыто | скрыто | видно |
| Что будет дальше | скрыто | скрыто | скрыто |
| Конец серии | ничего: следующая просто начинается | карточка с обратным отсчётом | карточка с обратным отсчётом |
| «Успею за N минут?» | нельзя спросить | «да» или «нет», шагами по 5 минут | «да» или «нет», шагами по 5 минут |
| Точный остаток, серия и сезон | нельзя спросить | после подтверждения | после подтверждения |

# Перевод

Интерфейс поставляется на **русском** и **английском** языках, следует языку
системы и переопределяется в настройках. Переводы лежат прямо в репозитории,
это обычные JSON-каталоги, исходный из них `src/locales/en.json`. Чтобы добавить язык
или исправить перевод, откройте pull request с изменениями каталогов; см.
[CONTRIBUTING.md](CONTRIBUTING.md#translations).

# Установка

Murk работает на Linux и Windows и находится в статусе пре-релиза (v0.1.0).
Скачать можно из [последнего релиза](https://github.com/3Lord3/Murk/releases/latest):
там лежат все сборки и `SHA256SUMS`.

| Платформа | Файл | Примечание |
|---|---|---|
| Debian, Ubuntu | `.deb` | нужен `libmpv2` (mpv ≥ 0.36) |
| Fedora | `.rpm` | нужен `mpv-libs` |
| любой Linux | AppImage | самодостаточный, системный mpv не нужен |
| любой Linux | `.flatpak` | `flatpak install Murk_0.1.0_x86_64.flatpak` |
| Windows 10, 11 | `.exe` | установщик; рядом лежит и `.msi`, для развёртывания |

Дистрибутивам, где всё ещё `libmpv.so.1` (Ubuntu 22.04, Debian 12), подойдут
AppImage или Flatpak: они несут mpv в себе. В репозиториях дистрибутивов, на
Flathub и в winget Murk пока нет.

# Сборка

Обеим платформам нужны pnpm, Node 22 и стабильный Rust; всё остальное — libmpv,
и он должен быть **client API 2.x** (`libmpv.so.2`, mpv ≥ 0.36).

## Linux

```sh
./scripts/deps.sh --install   # системные библиотеки; --check только покажет их
pnpm install
pnpm tauri build              # либо `pnpm tauri dev`, чтобы сразу запустить
```

`deps.sh` знает названия пакетов для ALT, Debian, Ubuntu, Fedora и Arch.

## Windows

Здесь libmpv нет ни в одном пакетном менеджере, поэтому `scripts/deps.ps1`
скачивает его и собирает импорт-библиотеку, которая нужна компоновщику MSVC. Для
этого шага понадобятся сборочные инструменты Visual Studio (ради `lib.exe`) и
7-Zip, а тулчейн Rust должен быть MSVC.

```powershell
pwsh -File scripts/deps.ps1
$env:MPV_LIB_DIR = "$PWD\src-tauri\mpv\lib"
$env:PATH = "$PWD\src-tauri\mpv\bin;$env:PATH"   # отсюда dev-сборка возьмёт DLL
pnpm install
pnpm tauri build --bundles nsis          # либо `pnpm tauri dev`
```

Сборка читает `MPV_LIB_DIR`; без неё она останавливается и прямо об этом
сообщает. `libmpv-2.dll` кладётся в установщик, так что установленной копии
ничего в `PATH` не нужно.

## Пакеты

Готовые пакеты складываются в `src-tauri/target/release/bundle/`.

| Канал | Чем собрать |
|---|---|
| `.deb`, `.rpm` | `pnpm tauri build --bundles deb,rpm` |
| AppImage | `./scripts/appimage.sh` |
| Flatpak | [`packaging/flatpak/`](packaging/flatpak/README.md) |
| `.exe` (NSIS), `.msi` | `pnpm tauri build --bundles nsis,msi` |

AppImage собирайте скриптом, а не через `tauri build --bundles appimage`: хук
AppRun от Tauri принудительно выставляет `GDK_BACKEND=x11`, что затащило бы
видеоплеер в XWayland в любой Wayland-сессии, а скрипт это отменяет.

Зависимости пакетов прописаны вручную, а не выведены автоматически; рассуждение
целиком лежит в [src-tauri/PACKAGING.md](src-tauri/PACKAGING.md).

# Разработка

- [CONTRIBUTING.md](CONTRIBUTING.md): правила, и главное из них, *ничто, что
  может испортить сюжет, не пересекает границу.*
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md): архитектура, сборка, тесты.

# Лицензия

Murk — свободное программное обеспечение под лицензией **GPL-2.0-or-later**:
GNU General Public License версии 2 либо, по вашему выбору, любой более поздней
версии. Полный текст — в файле [LICENSE](LICENSE).