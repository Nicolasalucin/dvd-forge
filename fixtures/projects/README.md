# Fixtures

`minimal.json` es un proyecto NTSC 4:3 de ejemplo con:

- un menú raíz en VMGM con Reproducir todo y Episodios;
- un menú de selección de episodios en VTSM;
- un episodio de **2 horas** en los metadatos de planificación, con un capítulo
  y una pista AC-3;
- una política de presupuesto DVD-5.

Los archivos de `projects/assets/` se generan con:

```bash
./scripts/generate-fixtures.sh
```

El clip generado para el episodio dura 20 segundos, mientras que el metadato
del fixture declara 2 horas para poder validar un plan de llenado cercano al
99 %. `dvdforge-cli plan` usa los metadatos guardados por defecto; con
`--probe` refresca la duración real con FFprobe:

```bash
dvdforge-cli plan fixtures/projects/minimal.json -o build/plan.json
dvdforge-cli plan fixtures/projects/minimal.json --probe -o build/plan-probed.json
dvdforge-cli render fixtures/projects/minimal.json -o build/out
```

Las rutas, tamaños y hashes del JSON son metadatos de ejemplo hasta que la
Fase 1 importe los medios y los sustituya por valores reales.

`avchd-dvd5.json` es un proyecto AVCHD 720p23.976 de ejemplo para PS4/PS5 y
Blu-ray:

- dos episodios de 20 s y 15 s con audio AC-3;
- reproducción continua con capítulos en los límites e interior de episodios;
- H.264 High@4.1, VBV 24 Mbps y salida UDF 2.50 vía tsMuxeR;
- sin menús interactivos (el fixture declara `menus: []`).

```bash
dvdforge-cli plan fixtures/projects/avchd-dvd5.json --probe -o build/avchd-plan.json
dvdforge-cli render fixtures/projects/avchd-dvd5.json --mode avchd -o build/avchd
dvdforge-cli render fixtures/projects/avchd-dvd5.json --mode avchd --avchd-folder -o build/avchd
```

`bd25-menu.json` es el fixture de regresión para el menú HDMV IG nativo:

- BD-25, 720p23.976, H.264 High@4.1 y un episodio de 20 s;
- menú raíz estático con «Reproducir todo» y «Episodio 01»;
- el render crea `menus/clip/`, adjunta el clip IG al BDMV y genera un ISO
  UDF 2.50 con playlist de menú y `MovieObject` de First Play/Top Menu.

```bash
cargo test -p dvdforge-render --test render_smoke \
  renders_bd25_with_native_hdmv_menu -- --ignored
cargo test -p dvdforge-render --test render_smoke \
  renders_avchd_dvd5_with_native_hdmv_menu -- --ignored
```
