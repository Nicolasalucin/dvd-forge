# DVD Forge Desktop

Aplicación Tauri v2 con Svelte 5, TypeScript, Vite y Tailwind CSS 4.

## Requisitos

- Node.js 20.19+ (probado con Node 26).
- npm (o pnpm si se instala).
- Rust estable y las dependencias de Tauri v2 para Linux (`webkit2gtk-4.1`, GTK3).

## Desarrollo

```bash
npm install
npm run bindings   # regenera src/lib/api/bindings.ts desde Rust
npm run check      # svelte-check
npm run tauri dev  # ventana nativa con Vite
```

## Bindings

`src/lib/api/bindings.ts` se genera con `tauri-specta` a partir de los comandos
y tipos registrados en `src-tauri/src/lib.rs`. No editar a mano.

El binario `export-bindings` permite regenerarlo sin abrir la ventana:

```bash
cargo run -q -p dvdforge-desktop --bin export-bindings
```

## Comandos IPC

- `project_new(name, standard, aspect)`
- `project_save(path, project)`
- `project_open(path)`
- `media_probe_file(path)`
- `plan_calculate(project)`
- `burn_list_devices()`
- `render_start(project, outputDir, channel)`
- `burn_start(isoPath, device, speed, verify, eject, channel)`
- `open_folder(path)`
- `read_image_data_url(path)`
- `menu_save_background(projectId, menuId, dataUrl)`

La pantalla **Burn** llama a `burn_list_devices()` al arrancar y usa el backend
nativo de cada sistema: growisofs/xorriso en Linux, `drutil`/`hdiutil` en macOS
e IMAPI2 en Windows.

La pantalla **Render Queue** usa un `Channel<JobEvent>` y el pipeline compartido
`dvdforge-render`; antes de renderizar rasteriza el menú activo con
`exportMenu`, lo guarda con `menu_save_background` y el backend usa esa
composición (textos y miniaturas incluidos) como fondo del menú. Al terminar
habilita “Abrir carpeta” y propaga el ISO a la pantalla **Burn**, que usa un
`Channel<BurnProgress>` conectado a `LinuxBurner`.

El **Media Bin** persiste las importaciones en `Project.assets` y permite
convertir vídeos en episodios, asignar música de menú y usar imágenes como fondo
del lienzo. El editor exporta `background.png`, `highlight.png` y `select.png` a
resolución nativa con máscaras de 4 colores y coordenadas pares.

El nuevo proyecto permite elegir **DVD-Video Universal** o **AVCHD Alta
Definición**. En AVCHD se seleccionan resolución (1080p/1080i/720p/480i/576i) y
framerate válido; el editor de menús se deshabilita y la reproducción es continua
con capítulos.

## Estructura

```text
apps/desktop/
├── src/
│   ├── App.svelte
│   ├── app.css
│   ├── main.ts
│   └── lib/
│       ├── api/          bindings generados + cliente tipado
│       ├── editor/       CanvasStage, buttonLayout, capas Konva y exportMenu
│       ├── screens/      Home, NewProject, MediaBin, MenuEditor,
│       │                 EncodePlan, RenderQueue y Burn
│       ├── stores/       estado reactivo con runes
│       └── utils/        formato de bytes/duración
├── src-tauri/
│   ├── capabilities/     permisos de ventana, eventos y diálogos
│   ├── src/commands/     comandos IPC
│   ├── src/lib.rs        builder de Tauri Specta + export de bindings
│   └── tauri.conf.json
├── package.json
├── tsconfig.json
└── vite.config.ts
```
