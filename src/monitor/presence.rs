//! Modo Autenticador: monitor de presencia por cámara.
//!
//! Cada `presence_check_interval_secs` se comprueba si el rostro frente a la cámara
//! coincide con la plantilla registrada. Si pasan `presence_lock_after_secs` sin
//! reconocerlo (o con la cámara fallando), se congela la app y se muestra el
//! lockscreen. Estando bloqueada, al reconocer al usuario se le pide superar la
//! verificación de vida y se reanuda la app automáticamente. El PIN solo se habilita mientras
//! se reconoce el rostro registrado.

use crate::config::settings::Settings;
use super::camera_guard::{self, CameraUser};
use crate::gui::camera_prompt::{CameraPrompt, SuspendedNotice};
use crate::gui::lockscreen::PresenceLockscreen;
use crate::vision::liveness::{LivenessCheck, Verdict};
use crate::vision::worker::{Observation, VisionWorker};
use crate::vision::cosine_similarity;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::Application;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::process::Child;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// Intervalo rápido mientras la app está bloqueada (para el desafío de vida).
const LOCKED_INTERVAL: Duration = Duration::from_millis(500);

enum Phase {
    Unlocked { last_seen: Instant },
    Locked { screen: PresenceLockscreen, liveness: LivenessCheck },
}

/// Estado del acuerdo de uso de cámara con otras aplicaciones.
enum Share {
    Normal,
    /// Se está preguntando al usuario si cede la cámara a `user`.
    Prompting { prompt: CameraPrompt, user: CameraUser },
    /// Locker suspendido y cámara liberada mientras otra app la usa.
    /// `since`: desde cuándo; `seen`: la otra app ya apareció usando la cámara.
    Yielded { notice: SuspendedNotice, since: Instant, seen: bool },
}

/// Tolerancia desde el último reconocimiento facial para aceptar el SÍ.
const FACE_GRACE: Duration = Duration::from_secs(3);

/// Margen tras restaurar el Locker antes de exigir de nuevo el rostro.
/// Tiempo máximo de espera a que la app autorizada vuelva a pedir la cámara
/// (suele haber fallado ya contra AAL y hay que reabrirla).
const YIELD_WAIT: Duration = Duration::from_secs(90);

const RESTORE_GRACE: Duration = Duration::from_secs(8);

enum Action {
    None,
    Lock,
    Unlock,
}

struct Monitor {
    app: Application,
    target_app: String,
    child_rc: Rc<RefCell<Child>>,
    settings: Settings,
    template: Vec<f32>,
    worker: Option<VisionWorker>,
    phase: Phase,
    share: Share,
    /// PIDs a los que el usuario negó la cámara (no se vuelve a preguntar mientras sigan abiertos).
    denied: HashSet<u32>,
    _hold: gtk4::gio::ApplicationHoldGuard,
}

impl Monitor {
    fn pid(&self) -> u32 {
        self.child_rc.borrow().id()
    }

    fn check_interval(&self) -> Duration {
        Duration::from_secs(self.settings.presence_check_interval_secs.max(1) as u64)
    }

    fn lock_after(&self) -> Duration {
        Duration::from_secs(self.settings.presence_lock_after_secs.max(1) as u64)
    }

    fn recognized(&self, embedding: &[f32]) -> bool {
        cosine_similarity(&self.template, embedding) >= self.settings.face_match_threshold
    }

    /// ¿Se reconoció al usuario registrado hace muy poco (y la app está desbloqueada)?
    fn face_recent(&self) -> bool {
        // `last_seen` puede estar en el futuro (margen de restauración): eso no cuenta como rostro.
        matches!(&self.phase, Phase::Unlocked { last_seen }
            if *last_seen <= Instant::now() && last_seen.elapsed() <= FACE_GRACE)
    }

    /// Busca otras apps con la cámara abierta y avanza el acuerdo de uso.
    fn scan_camera(&mut self, this: &Weak<RefCell<Monitor>>) {
        if self.worker.is_none() {
            return;
        }
        let users = camera_guard::other_users(&self.settings.camera_device);
        let pids: HashSet<u32> = users.iter().map(|u| u.pid).collect();
        // Una denegación dura mientras siga vivo el proceso (aunque suelte y reabra el flujo).
        self.denied.retain(|p| Path::new(&format!("/proc/{p}")).exists());

        match &self.share {
            Share::Normal => {
                // Con la app bloqueada no se pregunta: la cámara sigue ocupada por AAL.
                if !matches!(self.phase, Phase::Unlocked { .. }) {
                    return;
                }
                if let Some(user) = users.into_iter().find(|u| !self.denied.contains(&u.pid)) {
                    eprintln!("[presence] {} (pid {}) intenta usar la cámara", user.name, user.pid);
                    let weak = this.clone();
                    let answer = Rc::new(move |yes: bool| {
                        // Se difiere: el callback puede dispararse con el monitor prestado.
                        let weak = weak.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(m) = weak.upgrade() {
                                m.borrow_mut().on_prompt_answer(yes);
                            }
                        });
                    });
                    let prompt = CameraPrompt::show(&self.app, &user.name, answer);
                    prompt.set_yes_enabled(self.face_recent());
                    self.share = Share::Prompting { prompt, user };
                }
            }
            // La pregunta permanece hasta que se responda: la otra app suele rendirse
            // en cuanto falla contra AAL, y aun así hay que decidir si se le cede.
            Share::Prompting { .. } => {}
            Share::Yielded { seen, since, .. } => {
                let (was_seen, waited) = (*seen, since.elapsed());
                // Solo cuenta como uso un flujo transmitiendo: la app que falló contra AAL
                // puede seguir conectada sin imagen y no debe contar ni como uso ni como cierre.
                if users.iter().any(|u| u.active) {
                    if let Share::Yielded { seen, .. } = &mut self.share {
                        *seen = true;
                    }
                } else if was_seen || waited >= YIELD_WAIT {
                    self.restore();
                }
            }
        }
    }

    fn on_prompt_answer(&mut self, yes: bool) {
        let share = std::mem::replace(&mut self.share, Share::Normal);
        let Share::Prompting { user, .. } = share else {
            self.share = share; // respuesta tardía de una pregunta ya cancelada
            return;
        };
        // El SÍ exige reconocer al usuario registrado en este instante.
        if !yes || !self.face_recent() {
            eprintln!("[presence] cámara denegada a {}", user.name);
            self.denied.insert(user.pid);
            return;
        }
        eprintln!("[presence] cámara cedida a {}: Locker suspendido", user.name);
        if let Some(w) = &self.worker {
            w.pause();
        }
        self.share = Share::Yielded {
            notice: SuspendedNotice::show(&self.app, &user.name),
            since: Instant::now(),
            seen: false,
        };
    }

    /// Termina la suspensión: recupera la cámara y reanuda la verificación.
    fn restore(&mut self) {
        if let Share::Yielded { notice, .. } = std::mem::replace(&mut self.share, Share::Normal) {
            notice.close();
        }
        eprintln!("[presence] cámara liberada: Locker restaurado");
        if let Some(w) = &self.worker {
            w.resume();
        }
        // PipeWire tarda unos segundos en soltar el dispositivo tras el cierre de la
        // otra app: se da margen para que la cámara vuelva sin bloquear por error.
        self.phase = Phase::Unlocked { last_seen: Instant::now() + RESTORE_GRACE };
    }

    /// Tarea periódica. Devuelve `false` cuando el monitor debe terminar.
    fn tick(&mut self, this: &Weak<RefCell<Monitor>>) -> bool {
        if self.worker.is_none() {
            return false;
        }
        if let Ok(Some(_)) = self.child_rc.borrow_mut().try_wait() {
            // La app objetivo terminó (también cubre el caso suspendido, sin observaciones).
            match std::mem::replace(&mut self.share, Share::Normal) {
                Share::Prompting { prompt, .. } => prompt.close(),
                Share::Yielded { notice, .. } => notice.close(),
                Share::Normal => {}
            }
            if let Phase::Locked { screen, .. } = &self.phase {
                screen.close();
            }
            self.worker = None;
            return false;
        }
        self.scan_camera(this);
        true
    }

    fn lock(&mut self, this: &Weak<RefCell<Monitor>>) {
        // Si había una pregunta abierta y el usuario ya no está, se deniega.
        if let Share::Prompting { prompt, user } = std::mem::replace(&mut self.share, Share::Normal) {
            prompt.close();
            self.denied.insert(user.pid);
        }
        eprintln!("[presence] usuario no detectado: congelando {}", self.target_app);
        super::freeze(self.pid());
        let weak = this.clone();
        let on_pin_unlock = Rc::new(move || {
            if let Some(m) = weak.upgrade() {
                m.borrow_mut().unlock();
            }
        });
        let screen = PresenceLockscreen::show(&self.app, &self.target_app, on_pin_unlock);
        if let Some(w) = &self.worker {
            w.set_interval(LOCKED_INTERVAL);
        }
        let liveness = LivenessCheck::new(self.settings.liveness_policy, self.settings.face_strong_threshold);
        self.phase = Phase::Locked { screen, liveness };
    }

    fn unlock(&mut self) {
        eprintln!("[presence] usuario verificado: reanudando {}", self.target_app);
        if let Phase::Locked { screen, .. } = &self.phase {
            screen.close();
        }
        super::resume(self.pid());
        if let Some(w) = &self.worker {
            w.set_interval(self.check_interval());
        }
        self.phase = Phase::Unlocked { last_seen: Instant::now() };
    }

    /// Procesa una observación. Devuelve `false` si el monitor debe terminar.
    fn handle(&mut self, this: &Weak<RefCell<Monitor>>, obs: Observation) -> bool {
        // Si la app objetivo terminó, apagamos cámara y monitor.
        if let Ok(Some(_)) = self.child_rc.borrow_mut().try_wait() {
            if let Phase::Locked { screen, .. } = &self.phase {
                screen.close();
            }
            self.worker = None;
            return false;
        }

        // Con la cámara cedida no hay verificación: lo gestiona `tick`/`scan_camera`.
        if matches!(self.share, Share::Yielded { .. }) {
            return true;
        }

        let seen = match &obs {
            Observation::Face { embedding, .. } => self.recognized(embedding),
            _ => false,
        };
        if std::env::var_os("AAL_DEBUG").is_some() {
            match &obs {
                Observation::Face { score, metrics, embedding } => eprintln!(
                    "[presence] rostro ancho={:.0}px score={score:.2} yaw={:+.2} sim={:.3} reconocido={seen}",
                    metrics.face_width, metrics.yaw, cosine_similarity(&self.template, embedding)
                ),
                Observation::NoFace => eprintln!("[presence] sin rostro"),
                Observation::Error(_) => {}
            }
        }
        if let Observation::Error(e) = &obs {
            eprintln!("[presence] {e}");
        }

        if let Share::Prompting { prompt, .. } = &self.share {
            prompt.set_yes_enabled(self.face_recent());
        }

        let lock_after = self.lock_after();
        let action = match &mut self.phase {
            Phase::Unlocked { last_seen } => {
                if seen {
                    *last_seen = Instant::now();
                    Action::None
                } else if last_seen.elapsed() >= lock_after {
                    Action::Lock
                } else {
                    Action::None
                }
            }
            Phase::Locked { screen, liveness } => {
                screen.set_face_recognized(seen);
                match obs {
                Observation::Face { metrics, embedding, .. } => {
                    let similarity = cosine_similarity(&self.template, &embedding);
                    match liveness.feed(&metrics, similarity, seen) {
                        Verdict::Passed => Action::Unlock,
                        Verdict::Pending(text) => {
                            screen.set_status(&text);
                            Action::None
                        }
                        Verdict::Failed => {
                            screen.set_status("Verificación fallida. Muestra tu rostro registrado para usar el PIN.");
                            liveness.reset();
                            Action::None
                        }
                    }
                }
                Observation::NoFace => {
                    liveness.reset();
                    screen.set_status("Buscando al usuario registrado…");
                    Action::None
                }
                Observation::Error(e) => {
                    liveness.reset();
                    screen.set_status(&format!("Cámara no disponible: {e}\nNo se puede desbloquear sin verificar el rostro."));
                    Action::None
                }
                }
            }
        };

        match action {
            Action::Lock => self.lock(this),
            Action::Unlock => self.unlock(),
            Action::None => {}
        }
        true
    }
}

/// Arranca el monitor de presencia para la app ya lanzada.
// `MainContext::channel` está deprecado en glib ≥0.19; con gtk4 0.7 (glib 0.18) es la vía disponible.
#[allow(deprecated)]
pub fn start_presence_monitor(
    app: Application,
    target_app: String,
    child_rc: Rc<RefCell<Child>>,
    settings: Settings,
    template: Vec<f32>,
) {
    let (tx, rx) = glib::MainContext::channel::<Observation>(glib::Priority::DEFAULT);
    let interval = Duration::from_secs(settings.presence_check_interval_secs.max(1) as u64);
    let device = settings.camera_device.clone();
    let worker = VisionWorker::spawn(settings.camera_device.clone(), Settings::models_dir(), interval, tx);

    let monitor = Rc::new(RefCell::new(Monitor {
        _hold: app.hold(),
        app,
        target_app,
        child_rc,
        settings,
        template,
        worker: Some(worker),
        phase: Phase::Unlocked { last_seen: Instant::now() },
        share: Share::Normal,
        denied: HashSet::new(),
    }));

    let weak = Rc::downgrade(&monitor);

    // Tarea periódica: detecta usos de cámara y el fin de la app objetivo.
    let tick_weak = weak.clone();
    glib::timeout_add_seconds_local(1, move || {
        let Some(m) = tick_weak.upgrade() else { return glib::ControlFlow::Break };
        let keep = m.borrow_mut().tick(&tick_weak);
        if keep { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });

    watch_camera_opens(&device, weak.clone());

    rx.attach(None, move |obs| {
        let keep_going = monitor.borrow_mut().handle(&weak, obs);
        if keep_going {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

/// Avisa de cada apertura del dispositivo de cámara (inotify `IN_OPEN`) para
/// detectar intentos de otras apps al instante, antes del sondeo periódico.
fn watch_camera_opens(device: &str, weak: Weak<RefCell<Monitor>>) {
    use inotify::{Inotify, WatchMask};
    use std::os::fd::AsRawFd;

    let Ok(mut ino) = Inotify::init() else { return };
    if ino.watches().add(device, WatchMask::OPEN).is_err() {
        return;
    }
    let fd = ino.as_raw_fd();
    glib::source::unix_fd_add_local(fd, glib::IOCondition::IN, move |_, _| {
        let mut buf = [0u8; 1024];
        let _ = ino.read_events(&mut buf).map(|events| events.count());
        let Some(m) = weak.upgrade() else { return glib::ControlFlow::Break };
        m.borrow_mut().scan_camera(&weak);
        // El otro proceso puede tardar unos ms en dejar el descriptor abierto: se reintenta.
        for ms in [100u64, 300, 700] {
            let w = weak.clone();
            glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                if let Some(m) = w.upgrade() {
                    m.borrow_mut().scan_camera(&w);
                }
            });
        }
        glib::ControlFlow::Continue
    });
}
