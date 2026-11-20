//! Enrolamiento facial: captura varias muestras (de frente y girando la cabeza)
//! para construir la plantilla del usuario. El giro de cabeza también actúa como
//! verificación de vida durante el registro. No se guarda ninguna imagen.

use crate::config::settings::Settings;
use crate::vision::liveness::{is_frontal, is_turned};
use crate::vision::worker::{Observation, VisionWorker};
use crate::vision::{cosine_similarity, mean_template};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box, Button, Label, Orientation};
use libadwaita::Application;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const CAPTURE_INTERVAL: Duration = Duration::from_millis(400);
/// Confianza mínima del detector para aceptar una muestra.
const MIN_SCORE: f32 = 0.6;
/// Similitud mínima con las muestras ya aceptadas (evita mezclar personas).
const MIN_CONSISTENCY: f32 = 0.3;

const FRONT_SAMPLES: usize = 5;
const TURN_SAMPLES: usize = 3;
const FRONT_AGAIN_SAMPLES: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Front,
    Turn,
    FrontAgain,
    Done,
}

struct Enroll {
    step: Step,
    collected: usize,
    samples: Vec<Vec<f32>>,
    worker: Option<VisionWorker>,
}

impl Enroll {
    fn target(&self) -> usize {
        match self.step {
            Step::Front => FRONT_SAMPLES,
            Step::Turn => TURN_SAMPLES,
            Step::FrontAgain => FRONT_AGAIN_SAMPLES,
            Step::Done => 0,
        }
    }

    fn instruction(&self) -> String {
        let n = format!("({}/{})", self.collected, self.target());
        match self.step {
            Step::Front => format!("Mira al frente a la cámara {n}"),
            Step::Turn => format!("Gira la cabeza hacia un lado {n}"),
            Step::FrontAgain => format!("Mira al frente otra vez {n}"),
            Step::Done => "Rostro registrado correctamente".to_string(),
        }
    }

    fn accepts(&self, yaw: f32, embedding: &[f32]) -> bool {
        let pose_ok = match self.step {
            Step::Front | Step::FrontAgain => is_frontal(yaw),
            Step::Turn => is_turned(yaw),
            Step::Done => false,
        };
        if !pose_ok {
            return false;
        }
        if self.samples.is_empty() {
            return true;
        }
        let current = mean_template(&self.samples);
        cosine_similarity(&current, embedding) >= MIN_CONSISTENCY
    }

    fn push(&mut self, embedding: Vec<f32>) {
        self.samples.push(embedding);
        self.collected += 1;
        if self.collected >= self.target() {
            self.collected = 0;
            self.step = match self.step {
                Step::Front => Step::Turn,
                Step::Turn => Step::FrontAgain,
                Step::FrontAgain | Step::Done => Step::Done,
            };
        }
    }
}

/// Muestra la ventana de enrolamiento. `on_finished(true)` si se guardó la plantilla.
// `MainContext::channel` está deprecado en glib ≥0.19; con gtk4 0.7 (glib 0.18) es la vía disponible.
#[allow(deprecated)]
pub fn show_enroll_window(app: &Application, on_finished: Rc<dyn Fn(bool)>) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Registrar rostro")
        .default_width(420)
        .default_height(220)
        .modal(true)
        .build();

    let vbox = Box::new(Orientation::Vertical, 12);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let title = Label::new(Some("Colócate frente a la cámara con buena iluminación."));
    let instruction = Label::new(Some("Iniciando cámara…"));
    instruction.add_css_class("title-3");
    let detail = Label::new(Some("Solo se guarda una firma numérica del rostro, nunca imágenes."));
    detail.add_css_class("dim-label");
    detail.set_wrap(true);
    let cancel_btn = Button::with_label("Cancelar");

    vbox.append(&title);
    vbox.append(&instruction);
    vbox.append(&detail);
    vbox.append(&cancel_btn);
    window.set_child(Some(&vbox));
    window.present();

    let settings = Settings::load();
    let (tx, rx) = glib::MainContext::channel::<Observation>(glib::Priority::DEFAULT);
    let worker = VisionWorker::spawn(settings.camera_device, Settings::models_dir(), CAPTURE_INTERVAL, tx);

    let state = Rc::new(RefCell::new(Enroll {
        step: Step::Front,
        collected: 0,
        samples: Vec::new(),
        worker: Some(worker),
    }));

    let finished = Rc::new(RefCell::new(false));

    {
        let state = state.clone();
        let window = window.clone();
        let finished = finished.clone();
        let on_finished = on_finished.clone();
        cancel_btn.connect_clicked(move |_| {
            state.borrow_mut().worker = None;
            if !*finished.borrow() {
                *finished.borrow_mut() = true;
                on_finished(false);
            }
            window.close();
        });
    }

    {
        let state = state.clone();
        let finished = finished.clone();
        let on_finished = on_finished.clone();
        window.connect_close_request(move |_| {
            state.borrow_mut().worker = None;
            if !*finished.borrow() {
                *finished.borrow_mut() = true;
                on_finished(false);
            }
            glib::Propagation::Proceed
        });
    }

    let window_c = window.clone();
    rx.attach(None, move |obs| {
        // Todo el trabajo con `state` se hace dentro de este bloque para soltar el
        // préstamo antes de cerrar la ventana (su close_request también toma `state`).
        let outcome: Option<bool> = {
            let mut st = state.borrow_mut();
            if st.worker.is_none() {
                return glib::ControlFlow::Break;
            }
            match obs {
                Observation::Face { score, yaw, embedding } => {
                    if score >= MIN_SCORE && st.accepts(yaw, &embedding) {
                        st.push(embedding);
                    }
                    instruction.set_text(&st.instruction());
                    if st.step == Step::Done {
                        let template = mean_template(&st.samples);
                        st.worker = None; // Apagar cámara cuanto antes.
                        match crate::security::save_face_template(&template) {
                            Ok(()) => Some(true),
                            Err(e) => {
                                instruction.set_text(&format!("No se pudo guardar la plantilla: {e}"));
                                Some(false)
                            }
                        }
                    } else {
                        None
                    }
                }
                Observation::NoFace => {
                    instruction.set_text("No se detecta ningún rostro. Acércate a la cámara.");
                    None
                }
                Observation::Error(e) => {
                    instruction.set_text(&format!("Cámara no disponible: {e}"));
                    None
                }
            }
        };

        match outcome {
            None => glib::ControlFlow::Continue,
            Some(ok) => {
                *finished.borrow_mut() = true;
                on_finished(ok);
                if ok {
                    window_c.close();
                }
                glib::ControlFlow::Break
            }
        }
    });
}
