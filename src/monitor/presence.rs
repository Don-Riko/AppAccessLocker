//! Modo Autenticador: monitor de presencia por cámara.
//!
//! Cada `presence_check_interval_secs` se comprueba si el rostro frente a la cámara
//! coincide con la plantilla registrada. Si pasan `presence_lock_after_secs` sin
//! reconocerlo (o con la cámara fallando), se congela la app y se muestra el
//! lockscreen. Estando bloqueada, al reconocer al usuario se le pide superar la
//! verificación de vida y se reanuda la app automáticamente. El PIN solo se habilita mientras
//! se reconoce el rostro registrado.

use crate::config::settings::Settings;
use crate::gui::lockscreen::PresenceLockscreen;
use crate::vision::liveness::{LivenessCheck, Verdict};
use crate::vision::worker::{Observation, VisionWorker};
use crate::vision::cosine_similarity;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita::Application;
use std::cell::RefCell;
use std::process::Child;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

/// Intervalo rápido mientras la app está bloqueada (para el desafío de vida).
const LOCKED_INTERVAL: Duration = Duration::from_millis(500);

enum Phase {
    Unlocked { last_seen: Instant },
    Locked { screen: PresenceLockscreen, liveness: LivenessCheck },
}

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

    fn lock(&mut self, this: &Weak<RefCell<Monitor>>) {
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
    }));

    let weak = Rc::downgrade(&monitor);
    rx.attach(None, move |obs| {
        let keep_going = monitor.borrow_mut().handle(&weak, obs);
        if keep_going {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}
