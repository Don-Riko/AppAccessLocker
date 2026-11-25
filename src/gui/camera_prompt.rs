//! Diálogos del acuerdo de uso de cámara con otras aplicaciones.

use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box, Button, Label, Orientation};
use libadwaita::Application;
use std::cell::Cell;
use std::rc::Rc;

fn base_window(app: &Application, title: &str) -> (ApplicationWindow, Box) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(420)
        .resizable(false)
        .build();
    let vbox = Box::new(Orientation::Vertical, 12);
    vbox.set_margin_top(20);
    vbox.set_margin_bottom(20);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);
    window.set_child(Some(&vbox));
    (window, vbox)
}

/// Pregunta si se cede la cámara. `on_answer(true)` = SÍ, `on_answer(false)` = NO.
pub struct CameraPrompt {
    window: ApplicationWindow,
    yes_btn: Button,
    note: Label,
}

impl CameraPrompt {
    pub fn show(app: &Application, other_app: &str, on_answer: Rc<dyn Fn(bool)>) -> Self {
        let (window, vbox) = base_window(app, "Solicitud de uso de cámara");
        let text = Label::new(Some(&format!(
            "Se detectó otro intento de uso de la cámara por parte de {other_app}.\n\
             ¿Permitir que {other_app} use la cámara?\n\
             Si aceptas, AppAccessLocker se desactivará temporalmente."
        )));
        text.set_wrap(true);
        text.set_justify(gtk4::Justification::Center);
        let note = Label::new(None);
        note.add_css_class("dim-label");
        note.set_wrap(true);

        let yes_btn = Button::with_label("SÍ (desactivar Locker temporalmente)");
        yes_btn.add_css_class("destructive-action");
        let no_btn = Button::with_label("NO (mantener Locker)");
        no_btn.add_css_class("suggested-action");

        // La respuesta se emite una sola vez; cerrar la ventana sin responder equivale a NO.
        let answered = Rc::new(Cell::new(false));
        let answer = {
            let answered = answered.clone();
            move |yes: bool| {
                if !answered.replace(true) {
                    on_answer(yes);
                }
            }
        };
        let answer = Rc::new(answer);
        let (a, w) = (answer.clone(), window.clone());
        yes_btn.connect_clicked(move |_| {
            a(true);
            w.close();
        });
        let (a, w) = (answer.clone(), window.clone());
        no_btn.connect_clicked(move |_| {
            a(false);
            w.close();
        });
        let a = answer;
        window.connect_close_request(move |_| {
            a(false);
            gtk4::glib::Propagation::Proceed
        });

        vbox.append(&text);
        vbox.append(&yes_btn);
        vbox.append(&no_btn);
        vbox.append(&note);
        window.present();
        Self { window, yes_btn, note }
    }

    /// Habilita o deshabilita el SÍ (solo si se reconoce al usuario registrado).
    pub fn set_yes_enabled(&self, enabled: bool) {
        self.yes_btn.set_sensitive(enabled);
        self.note.set_text(if enabled {
            ""
        } else {
            "Para aceptar debes estar frente a la cámara y ser reconocido."
        });
    }

    pub fn close(&self) {
        self.window.close();
    }
}

/// Aviso persistente mientras el Locker está desactivado temporalmente.
pub struct SuspendedNotice {
    window: ApplicationWindow,
}

impl SuspendedNotice {
    pub fn show(app: &Application, other_app: &str) -> Self {
        let (window, vbox) = base_window(app, "Locker desactivado temporalmente");
        let text = Label::new(Some(&format!(
            "AppAccessLocker está desactivado temporalmente.\n\
             La cámara ya está libre para {other_app}.\n\
             Si no muestra imagen, ciérrala y ábrela de nuevo.\n\
             El Locker se restaurará al cerrarse ese uso."
        )));
        text.set_wrap(true);
        text.set_justify(gtk4::Justification::Center);
        vbox.append(&text);
        window.present();
        Self { window }
    }

    pub fn close(&self) {
        self.window.close();
    }
}
