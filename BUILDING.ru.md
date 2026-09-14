# Сборка

[English](BUILDING.md) · [Русский](BUILDING.ru.md)

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

[← К README](README.ru.md)
