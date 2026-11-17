# Guía de uso de Kinodisc

Kinodisc convierte tus vídeos en un DVD, AVCHD o Blu-ray con menús que
funcionan en el reproductor del living. Esta guía sigue los pasos en orden.
Pulsa **F1** en cualquier momento para ver la guía interactiva.

## 1. Antes de empezar

- **Windows:** en la pantalla de bienvenida pulsa **Descargar y preparar**.
  Kinodisc descarga una sola vez FFmpeg y DVDAuthor (unos 120 MB), comprueba
  su SHA-256 y los guarda en tu carpeta de usuario.
- **Linux, macOS, FreeBSD:** instala `ffmpeg` y `dvdauthor` con tu gestor de
  paquetes (ver el README).
- La barra de estado (abajo a la derecha) dice **Herramientas listas** o
  **Faltan herramientas**; haz clic para ver cada programa e instalar lo que falte.

El tema sigue al sistema; **Ver › Tema** fuerza claro u oscuro.
**Ver › Idioma** cambia el idioma de la interfaz. Una vez al día Kinodisc le
pregunta a GitHub si hay una versión nueva y lo avisa en la barra de estado;
se desactiva en **Ayuda › Buscar actualizaciones al iniciar**.

## 2. Crear un proyecto

**Archivo › Nuevo proyecto** (Ctrl+N) o la tarjeta **Nuevo proyecto**:

- **DVD-Video universal** funciona casi en cualquier lado. El sistema de TV
  (PAL/NTSC) se elige según tu región; la pantalla es 16:9 salvo que elijas 4:3.
- **Blu-ray / AVCHD (alta definición)** para reproductores Blu-ray y PlayStation:
  - 1080p 23,976/24 para cine, 25/29,97 para vídeo casero (se graba como PsF y
    funciona en todo reproductor Blu-ray), 50/59,94 para movimiento muy fluido
    (AVCHD 2.0; algunos reproductores antiguos no lo admiten).

Después, el **botón del disco** a la derecha de las pestañas (por ejemplo
`DVD-5 · PAL · 16:9`) abre los ajustes: formato, capacidad (DVD-5/9,
BD-25/50), sistema de TV, pantalla, región y reproductor de destino.

## 3. Añadir vídeos

**Importar vídeos…** en el proyecto vacío, **Importar** en la barra (Ctrl+I) o
arrastra los archivos a la ventana. Cada vídeo es un episodio en la tira
**Episodios** de abajo; arrastra los fotogramas para cambiar el orden.

En **Medios** (izquierda) cada archivo tiene **Añadir**; el botón **⋯** ofrece
los demás usos (miniatura de botón, fondo del menú, fondo animado, música).

### Recorte y capítulos

En **Vídeos**, la tira bajo el reproductor muestra fotogramas de todo el vídeo:
arrastra los tiradores azules o usa **Marcar inicio / Marcar fin**. Los
capítulos son marcas ámbar; añádelos en el cabezal o con *Cada 5 min*, *Cada
10 min*, *4 partes* o **Por escenas** (un capítulo en cada cambio de escena),
o impórtalos del archivo.

### Arreglar la imagen

La tarjeta **Imagen**, debajo del reproductor, arregla un vídeo antes de que
vaya al disco; la vista previa muestra el fotograma del cabezal tal como
quedará:

- **Orientación**: girar a la izquierda/derecha y espejo (vídeos del celular
  grabados de costado).
- **Recortar bordes**: escribe los píxeles o pulsa **Detectar bordes negros**.
- **Brillo, contraste, saturación** (doble clic en un control lo restablece).
- **Reducir ruido** (suave/fuerte) y **Desentrelazar** (automático cuando un
  vídeo entrelazado de filmadora va a un Blu-ray progresivo).
- **Laterales del vídeo vertical**: barras negras o un **fondo desenfocado**
  hecho con el mismo vídeo.
- **Probar calidad** codifica 10 segundos igual que en el disco y los muestra
  al lado del original, con un veredicto.

### Unir vídeos

**Disco › Unir vídeos…** (o el botón del proyecto vacío) convierte varios
clips en un solo episodio, en el orden que elijas, con un capítulo al
principio de cada uno. Sirven tamaños y fps distintos y clips sin sonido.

### Vídeos que no hace falta recodificar (Blu-ray/AVCHD)

Si un vídeo ya coincide con el disco (`.m2ts` de videocámara, o H.264 con la
resolución y los fps del proyecto), Kinodisc **lo copia sin recodificar**:
mucho más rápido y sin pérdida. El campo **Vídeo** de las propiedades del
episodio dice si se copia o se recodifica, y por qué. Para copiarse, el
episodio no puede estar recortado al inicio.

### Presentaciones de fotos

**Fotos** en la barra: añade imágenes, ordénalas, elige los segundos por foto,
el fundido y música opcional. La presentación se crea como un vídeo y se añade
como episodio.

### Importar un disco existente

**Disco** en la barra abre el inspector: abre un ISO o una carpeta
VIDEO_TS/BDMV para ver títulos, capítulos y pistas, e **Importar como
episodios** si no está cifrado. Los discos cifrados (comerciales) se pueden
ver pero no importar.

## 4. Diseñar el menú

En **Menú**:

- **Plantillas** aplica uno de 15 diseños (boda, cumpleaños, viaje,
  deportes, vacaciones, cine antiguo, naturaleza, documental, infantil,
  elegante…) con un botón por episodio; agrega páginas solo si hace falta.
  Cada plantilla trae su color de fondo y su decoración; si eliges una imagen
  de fondo propia, reemplaza el color. **Menús de escenas**, dentro del panel de plantillas,
  crea un menú con un botón por capítulo.
- **Botón** añade un botón; **Texto** añade un título o cualquier texto.
- Arrastra para mover: aparecen guías rosas cuando un botón se alinea con otro
  o con el centro. Con clic derecho sobre un botón: duplicar, eliminar,
  centrar, traer al frente, alinear o distribuir todos.
- Propiedades (derecha): texto con **selector de fuentes**, tamaño, peso y
  colores; **Al pulsar** (reproducir episodio, todo, ir a un menú, a un
  capítulo, a una pista de audio…); miniatura; y **Navegación con el mando**
  (a qué botón lleva cada flecha).
- **Fondo** y **Música** en la barra del menú; *Normal / Resaltado /
  Selección* muestran cómo se ven los botones al moverse con el mando.

## 5. Audio y subtítulos

En **Audio y subtítulos** elige el idioma, formato y bitrate de cada pista,
marca la pista por defecto y corrige desfases (±10 s). Añade subtítulos desde
archivos `.srt`, `.ass`, `.vtt` o `.sup`; el estilo se ajusta por pista.

## 6. Probar el disco

**Vista previa** muestra el menú en una TV con un mando virtual: flechas, OK,
Menú, Título y Volver funcionan como en un reproductor real, incluidos
«reproducir todo» y los capítulos.

## 7. Producir y grabar

**Producir disco** (Ctrl+R) ofrece:

- **Solo ISO**, **Carpeta del disco** (VIDEO_TS/BDMV para reproductores, USB o
  Kodi), **ISO + grabar** o **ISO existente**.
- **Discos:** un solo disco, varios discos (llenando cada uno) o uno por
  episodio. Los discos divididos muestran `1 / 3`, `2 / 3`… en el menú.

El verificador estructural revisa cada imagen antes de permitir grabarla.

**Apagar la computadora al terminar** la apaga al acabar el trabajo (una
cuenta regresiva de 60 segundos permite cancelarlo). En cualquier caso,
Kinodisc suena y parpadea en la barra de tareas al terminar.

### Carátula y etiqueta

**Disco › Carátula y etiqueta…** crea la carátula de la caja (DVD 273×183 mm,
Blu-ray 262×148 mm, con lomo) y la etiqueta redonda del disco (118 mm, centro
en blanco) con los colores, la letra y las miniaturas del menú. Edita los
textos y pulsa **Guardar PNG** (300 ppp; imprímelo al 100 %) o **Imprimir**.

### Si la producción se corta a la mitad

Pulsa **Producir disco** otra vez: los episodios ya codificados se reutilizan.

### Varios proyectos seguidos

**Disco › Cola de discos…**: añade proyectos guardados y pulsa **Generar
todo**; los que esperan siguen en la cola aunque cierres la app.

### Llevar o compartir un proyecto

**Archivo › Empaquetar con los vídeos…** guarda el proyecto y todo lo que usa
en un archivo `.kinodisc`. Ábrelo en la otra computadora y elige una carpeta
vacía; cada archivo se comprueba (SHA-256).

### Cambios sin guardar

Deshacer/rehacer con Ctrl+Z / Ctrl+Y. Un punto junto al nombre del proyecto
indica cambios sin guardar; Kinodisc pregunta antes de perderlos y guarda una
copia de recuperación cada 30 segundos por si la computadora se apaga.

## 8. Teclado

| Teclas | Acción |
|---|---|
| Ctrl+N / Ctrl+O / Ctrl+S | Nuevo, abrir, guardar (Ctrl+Mayús+S: guardar como) |
| Ctrl+Z / Ctrl+Y | Deshacer / rehacer |
| Ctrl+I | Importar medios |
| Ctrl+D | Duplicar el botón seleccionado |
| Ctrl+R | Producir disco |
| Flechas (Mayús = más rápido) | Mover el botón o texto seleccionado |
| `[` `]` | Botón anterior / siguiente |
| Supr / Esc | Eliminar / deseleccionar |
| F1 / F11 | Guía / pantalla completa |

## 9. Problemas comunes

| Problema | Solución |
|---|---|
| «Faltan herramientas» | Haz clic en la barra de estado e instálalas; en Linux/macOS ejecuta el comando que aparece. |
| Tras actualizar se reinstalan las herramientas | Kinodisc 1.0 comprueba cada archivo; las copias de versiones anteriores se descargan de nuevo una vez. |
| «No cabe» | Baja la calidad, elige un disco más grande o produce varios discos. |
| «Faltan N archivo(s) de este proyecto» | En Medios, pulsa **Buscar…** en el archivo marcado *No se encuentra*; los demás que estén en la misma carpeta nueva se encuentran solos. |
| Un botón no se puede mover ahí | Los botones necesitan 4 px de separación (regla de DVD/Blu-ray). |
| Aviso de audio | El registro de producción dice qué pista se ajustó y por qué. |

## 10. Herramientas avanzadas

**Ver › Herramientas avanzadas** muestra Animación (menús y miniaturas en
movimiento), Mapa VM (cada salto que puede hacer el reproductor) y Calidad
VBR (bitrate por episodio).
