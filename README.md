# AppAccessLocker (AAL)

Wrapper nativo para **Fedora / GNOME** que exige un PIN antes de abrir una
aplicación y la **congela** cuando dejas de usarla, volviendo a pedir
autenticación para reanudarla. Tiene dos modos de bloqueo:

| Modo | Qué vigila | Cuándo bloquea | Cómo desbloquea |
|---|---|---|---|
| **Clásico** (por defecto) | Inactividad de teclado/ratón (`org.gnome.Mutter.IdleMonitor`) | 30 s sin actividad | PIN |
| **Autenticador** | La **cámara**: ¿está el usuario registrado frente al equipo? | 3 s sin reconocerte (comprueba cada 1 s) | **Automático** al reconocerte de frente; PIN como respaldo |

Cómo funciona: en lugar de lanzar la app directamente, se lanza a través del
binario `app-access-locker`, que muestra una ventana de login. Si el PIN es
correcto, arranca la app real y monta el monitor del modo activo. Al bloquear,
envía `SIGSTOP` al proceso y muestra un lockscreen a pantalla completa; al
autenticar envía `SIGCONT` y reanuda.

```
usuario → app-access-locker /ruta/app [args] → [PIN] → app real
                                                  ↑ (inactividad | usuario ausente) SIGSTOP → lockscreen → SIGCONT
```

Ejecutado **sin argumentos**, abre la ventana de **configuración**: elegir el
modo, registrar/eliminar el rostro y ajustar el candado de confirmación.

---

## 1. Requisitos

Solo funciona en una **sesión GNOME** (usa el IdleMonitor de Mutter) sobre
Fedora o equivalente.

**Compilación:**
```bash
sudo dnf install cargo rustc gtk4-devel libadwaita-devel pkgconf-pkg-config
```

**Ejecución:**
```bash
sudo dnf install gnome-keyring        # proveedor Secret Service para el crate `keyring`
```
`busctl` (systemd) y GNOME/Mutter ya vienen con la sesión de escritorio.

**Modo Autenticador (opcional):**
- Una webcam V4L2 que ofrezca MJPG (prácticamente todas las integradas). Se
  accede a `/dev/video0` directamente; no hace falta pertenecer al grupo
  `video` en Fedora (logind da acceso al usuario de la sesión).
- Los dos modelos ONNX del paquete `buffalo_sc` de insightface (≈16 MB), en
  `~/.local/share/appaccesslocker/models/`:
  ```bash
  mkdir -p ~/.local/share/appaccesslocker/models && cd $_
  curl -L -o buffalo_sc.zip https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_sc.zip
  unzip -o buffalo_sc.zip && rm buffalo_sc.zip
  ls   # det_500m.onnx  w600k_mbf.onnx
  ```
- **Sin dependencias nativas nuevas**: la captura (V4L2) y la inferencia (ONNX)
  van en Rust puro (`linuxvideo`, `tract-onnx`). No requiere OpenCV, clang,
  libv4l ni onnxruntime.

---

## 2. Alojamiento del proyecto (clonar / exportar)

```bash
git clone https://github.com/Don-Riko/AppAccessLocker.git
cd AppAccessLocker
```

Para exportarlo a otra máquina basta con clonar el repo y recompilar — no hay
estado embebido en el binario. El PIN se guarda en el **keyring del usuario**
(Secret Service), no en el repo, así que cada equipo/usuario define el suyo en
el primer arranque.

---

## 3. Compilación y despliegue del binario

```bash
cargo build --release          # genera target/release/app-access-locker
```

Copia el binario a un directorio en el `PATH` (el `.desktop` invoca
`app-access-locker` sin ruta absoluta):

```bash
# Opción usuario (recomendada, sin sudo):
install -Dm755 target/release/app-access-locker ~/.local/bin/app-access-locker

# Opción sistema (para todos los usuarios):
sudo install -Dm755 target/release/app-access-locker /usr/local/bin/app-access-locker
```

Asegúrate de que `~/.local/bin` esté en el `PATH` (en Fedora ya lo está por
defecto para sesiones de login). Verifica:

```bash
command -v app-access-locker
```

---

## 4. Configuración inicial

En el **primer arranque**, si no hay PIN configurado, se abre la ventana de
configuración de PIN:

```bash
app-access-locker            # sin argumentos → setup del PIN / ventana de configuración
```

Introduce un PIN y pulsa **Guardar PIN**. Se almacena en el keyring del usuario
mediante `argon2` (hash) + `keyring` (Secret Service). Reinicia tras
configurarlo.

Con el PIN ya creado, `app-access-locker` sin argumentos abre la **ventana de
configuración**, con tres bloques:

1. **Modo de bloqueo**: Clásico (temporizador) o Autenticador (cámara).
2. **Candado de confirmación** (solo modo Autenticador): cuándo exigir un gesto
   aleatorio tras reconocerte. Ver [sección 6](#6-modo-autenticador-cámara).
3. **Rostro registrado**: registrar, volver a registrar o eliminar tu rostro.

### Registrar el rostro (modo Autenticador)

Pulsa **Registrar rostro…** y sigue las instrucciones en pantalla con buena
iluminación: 5 muestras **de frente**, 3 con la **cabeza girada** y 3 de frente
otra vez (unos 5–10 s). El giro también sirve como verificación de vida durante
el registro. Después marca **Autenticador** y pulsa **Guardar**.

**Privacidad:** solo se guarda una *firma numérica* del rostro (vector de 512
números) en el keyring, junto al PIN. **Nunca se almacenan imágenes.** La
cámara solo se enciende en modo Autenticador, mientras una app protegida está
abierta, y el lockscreen avisa de que está activa.

---

## 5. Enrutamiento de aplicaciones a bloquear

El bloqueo se aplica **por aplicación** sustituyendo su lanzador `.desktop` por
uno que pase la app real como argumento del locker:

```
Exec=app-access-locker /ruta/real/de/la/app %U
```

Un `.desktop` en `~/.local/share/applications/` con el **mismo nombre** que uno
del sistema lo sobrescribe solo para tu usuario.

**Ejemplo — bloquear Google Chrome:**

1. Averigua el `Exec` real de la app:
   ```bash
   grep -m1 '^Exec=' /usr/share/applications/google-chrome.desktop
   # Exec=/usr/bin/google-chrome-stable %U
   ```
2. Copia el lanzador a tu carpeta de usuario:
   ```bash
   cp /usr/share/applications/google-chrome.desktop ~/.local/share/applications/
   ```
3. Edita solo la línea `Exec=` para envolverla con el locker:
   ```ini
   Exec=app-access-locker /usr/bin/google-chrome-stable %U
   ```
4. Refresca la base de datos de lanzadores:
   ```bash
   update-desktop-database ~/.local/share/applications
   ```

A partir de ahora, abrir Chrome desde el menú pedirá el PIN. Repite el proceso
para cada app que quieras proteger. Para **desbloquear** una app, borra su
`.desktop` de `~/.local/share/applications/`.

**Ejemplo — bloquear Visual Studio Code:**

```bash
cp /usr/share/applications/code.desktop ~/.local/share/applications/
sed -i 's|^Exec= */usr/share/code/code %F|Exec=app-access-locker /usr/share/code/code %F|' \
    ~/.local/share/applications/code.desktop
update-desktop-database ~/.local/share/applications
```

> ⚠️ **Acciones del lanzador (`[Desktop Action …]`).** Muchos `.desktop` tienen
> entradas extra (p. ej. *New Window*, *New Incognito Window*, *New Empty
> Window*) con su **propio `Exec=`**, accesibles con clic derecho en el dock.
> Si no las envuelves también, abren la app **sin PIN**. Revisa todas:
> ```bash
> grep -n '^Exec=' ~/.local/share/applications/code.desktop
> ```
> y envuelve cada una, por ejemplo:
> ```ini
> Exec=app-access-locker /usr/share/code/code --new-window %F
> ```
> Al reemplazar el binario de `app-access-locker` no hay que tocar los
> `.desktop`: siguen apuntando a la misma ruta.

### Lanzador del propio AppAccessLocker (modo gestión)

El repo incluye `app-acces-locker.desktop` para tener AAL en el menú de apps:

```bash
install -Dm644 app-acces-locker.desktop \
  ~/.local/share/applications/app-acces-locker.desktop
update-desktop-database ~/.local/share/applications
```

---

## 6. Modo Autenticador (cámara)

Con una app protegida abierta, un hilo en segundo plano toma un fotograma cada
**1 s**, detecta el rostro (SCRFD-500m), lo alinea y calcula su firma
(MobileFaceNet, 512-d), y la compara con la registrada (similitud coseno,
umbral 0.40). Si pasan **3 s** sin reconocerte (o la cámara falla), la app se
congela y aparece el lockscreen.

**Desbloqueo automático**, en dos pasos:

1. **Identidad frontal**: debes verte de frente y ser reconocido en 3 lecturas
   seguidas (el muestreo sube a 2/s mientras está bloqueada).
2. **Candado de confirmación** (según política): una instrucción **aleatoria**
   en pantalla que una foto o un vídeo grabado no pueden acatar a tiempo:
   *Gira la cabeza hacia TU IZQUIERDA / TU DERECHA*, *Inclina la cabeza hacia
   un hombro* o *Acércate a la cámara*. Tienes 12 s; después debes volver a la
   posición neutra. Tras 3 instrucciones fallidas se remite al PIN.

| Política (`liveness_policy`) | Cuándo pide el gesto |
|---|---|
| `always` | Siempre |
| `uncertain` (por defecto) | Si la similitud media < `face_strong_threshold` (0.60), o al azar 1 de cada 4 |
| `never` | Nunca: basta reconocerte de frente |

Mientras sigas frente a la cámara, la app permanece abierta. El **PIN** siempre
funciona como respaldo en el lockscreen (cámara tapada, poca luz, etc.).

### Ajustes (`~/.config/appaccesslocker/settings.json`)

```json
{
  "lock_mode": "authenticator",          // "classic" | "authenticator"
  "camera_device": "/dev/video0",
  "presence_check_interval_secs": 1,     // cada cuánto se comprueba la presencia
  "presence_lock_after_secs": 3,         // segundos sin reconocerte antes de bloquear
  "face_match_threshold": 0.4,           // similitud mínima para "eres tú"
  "face_strong_threshold": 0.6,          // similitud a partir de la cual no se pide gesto (uncertain)
  "liveness_policy": "uncertain"         // "always" | "uncertain" | "never"
}
```

El modo y la política se cambian desde la ventana de configuración; el resto,
editando el archivo. Se aplica a las apps que se abran a partir de entonces.

### Ajuste del bloqueo por inactividad (modo Clásico)

El umbral está en `src/monitor/activity.rs`:

```rust
pub const IDLE_LIMIT_MS: u64 = 30_000;   // 30 s
```

Cámbialo y recompila para endurecer o relajar el tiempo de inactividad.

---

## 7. Replicar en otra máquina (checklist)

```bash
git clone https://github.com/Don-Riko/AppAccessLocker.git && cd AppAccessLocker
sudo dnf install cargo rustc gtk4-devel libadwaita-devel gnome-keyring
cargo build --release
install -Dm755 target/release/app-access-locker ~/.local/bin/app-access-locker
app-access-locker                      # configura el PIN inicial
# luego crea un .desktop wrapper por cada app (sección 5)

# Opcional, modo Autenticador:
mkdir -p ~/.local/share/appaccesslocker/models && cd $_
curl -L -o buffalo_sc.zip https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_sc.zip
unzip -o buffalo_sc.zip && rm buffalo_sc.zip && cd -
app-access-locker                      # Registrar rostro… → Autenticador → Guardar
```

El rostro se registra **por máquina y usuario** (vive en su keyring), igual que
el PIN.

---

## Limitaciones conocidas

- **Solo GNOME**: el monitor de inactividad depende de
  `org.gnome.Mutter.IdleMonitor`; en KDE/otros no detectará inactividad.
- El bloqueo evita el lanzamiento por el menú, no impide ejecutar el binario
  real directamente desde una terminal. Revisa también las `[Desktop Action]`
  de cada lanzador (sección 5).
- **Autenticador / anti-suplantación**: el candado de confirmación detiene
  fotografías y vídeos grabados, pero no a alguien que reproduzca un vídeo
  *interactivo* (p. ej. una videollamada en directo del usuario). El detector
  da solo 5 puntos (ojos, nariz, comisuras): no se pueden exigir parpadeos,
  guiños ni gestos de boca; haría falta un modelo de landmarks adicional.
- **Autenticador / condiciones**: con poca luz, contraluz fuerte o la cámara
  tapada el usuario no se reconoce y la app se bloquea (fail-closed); se
  desbloquea con el PIN.
- La verificación corre en CPU (~15 ms detección + ~35 ms firma por lectura);
  en modo bloqueado, a 2 lecturas/s, es un consumo pequeño pero continuo.
